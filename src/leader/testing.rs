pub(crate) type Id = u64;
pub(crate) type Signature = u64;
pub(crate) type Hash = &'static str;
pub(crate) type BlockNumber = u64;

pub mod chain {
	use super::*;
	use crate::std::collections::BTreeMap;
	pub const GENESIS_HASH: &'static str = "genesis";
	const NULL_HASH: &'static str = "NULL";

	#[derive(Debug)]
	struct BlockRecord {
		/// Block height
		number: BlockNumber,
		/// Parent hash
		parent: Hash,
	}

	/// A blockchain structure.
	#[derive(Debug)]
	pub struct DummyChain {
		inner: BTreeMap<Hash, BlockRecord>,
		finalized: (BlockNumber, Hash),
	}

	impl DummyChain {
		pub fn new() -> Self {
			let mut inner = BTreeMap::new();
			inner.insert(GENESIS_HASH, BlockRecord { number: 1, parent: NULL_HASH });
			DummyChain { inner, finalized: (1, GENESIS_HASH) }
		}

		/// Add a chain to current chain.
		pub fn push_blocks(&mut self, mut parent: Hash, blocks: &[Hash]) {
			if blocks.is_empty() {
				return
			}

			for i in blocks {
				self.push_block(parent, i);
				parent = i
			}
		}

		/// Add a block to current chain.
		pub fn push_block(&mut self, parent: Hash, block: Hash) {
			let block_number = self.inner.get(parent).unwrap().number + 1;
			self.inner.insert(block, BlockRecord { number: block_number, parent });
		}

		pub fn last_finalized(&self) -> (BlockNumber, Hash) {
			self.finalized
		}

		/// Get block after the last finalized block
		pub fn next_to_be_finalized(&self) -> Result<(BlockNumber, Hash), ()> {
			for (hash, record) in self.inner.iter().rev() {
				if record.number == self.finalized.0 + 1 {
					return Ok((record.number, hash.clone()))
				}
			}

			Err(())
		}

		/// Finalized a block.
		pub fn finalize_block(&mut self, block: Hash) -> bool {
			#[cfg(feature = "std")]
			log::trace!("finalize_block: {:?}", block);
			if let Some(b) = self.inner.get(&block) {
				#[cfg(feature = "std")]
				log::trace!("finalize block: {:?}", b);
				if self.inner.get(&b.parent).map(|p| p.number < b.number).unwrap_or(false) {
					self.finalized = (b.number, block);
					#[cfg(feature = "std")]
					log::info!("new finalized = {:?}", self.finalized);
					return true
				} else {
					panic!("block {} is not a descendent of {}", b.number, b.parent);
				}
			}

			false
		}
	}
}

#[cfg(feature = "std")]
pub mod environment {
	use std::{
		collections::HashMap,
		pin::Pin,
		sync::Arc,
		task::{Context, Poll},
		time::Duration,
	};

	use crate::{
		leader::{
			testing::chain::DummyChain,
			voter::{
				communicate::{RoundData, VoterData},
				Callback, Environment, GlobalMessageIn, GlobalMessageOut,
			},
			Error, FinalizedCommit, Message, SignedMessage,
		},
		voter::CommunicationIn,
	};

	use super::*;
	use futures::{
		channel::mpsc::{self, UnboundedReceiver, UnboundedSender},
		Future, Sink, SinkExt, Stream, StreamExt,
	};
	use futures_timer::Delay;
	use parking_lot::Mutex;

	/// p2p network data for a round.
	///
	/// Every node can send `Message` to the network, then it will be
	/// wrapped in `SignedMessage` and broadcast to all other nodes.
	struct BroadcastNetwork<M> {
		/// Receiver from every peer on the network.
		receiver: UnboundedReceiver<(Id, M)>,
		/// Raw sender to give a new node join the network.
		raw_sender: UnboundedSender<(Id, M)>,
		/// Peer's sender to send messages to.
		senders: Vec<(Id, UnboundedSender<M>)>,
		/// Broadcast history.
		history: Vec<M>,
		/// A validator hook is a hook to decide whether a message should be broadcast.
		/// By default, all messages are broadcast.
		validator_hook: Option<Box<dyn Fn(&M) -> () + Send + Sync>>,
		/// A routing table that decide whether a message should be sent to a peer.
		rule: Arc<Mutex<RoutingRule>>,
	}

	impl<M: Clone + std::fmt::Debug> BroadcastNetwork<M> {
		fn new(rule: Arc<Mutex<RoutingRule>>) -> Self {
			let (tx, rx) = mpsc::unbounded();
			BroadcastNetwork {
				receiver: rx,
				raw_sender: tx,
				senders: Vec::new(),
				history: Vec::new(),
				validator_hook: None,
				rule,
			}
		}

		fn register_validator_hook(&mut self, hook: Box<dyn Fn(&M) -> () + Send + Sync>) {
			self.validator_hook = Some(hook);
		}

		/// Add a node to the network for a round.
		fn add_node<N, F: Fn(N) -> M>(
			&mut self,
			id: Id,
			f: F,
		) -> (impl Stream<Item = Result<M, Error>>, impl Sink<N, Error = Error>) {
			log::trace!("BroadcastNetwork::add_node");
			// Channel from Network to the new node.
			let (tx, rx) = mpsc::unbounded();
			let messages_out = self
				.raw_sender
				.clone()
				.sink_map_err(|e| panic!("Error sending message: {:?}", e))
				.with(move |message| std::future::ready(Ok((id, f(message)))));

			// get history to the node.
			// for prior_message in self.history.iter().cloned() {
			// 	let _ = tx.unbounded_send(prior_message);
			// }

			// log::trace!("add_node: tx.isclosed? {}", tx.is_closed());
			self.senders.push((id, tx));

			log::trace!("BroadcastNetwork::add_node end.");
			(rx.map(Ok), messages_out)
		}

		// Do routing work
		fn route(&mut self, cx: &mut Context) -> Poll<()> {
			loop {
				// Receive item from receiver
				match Pin::new(&mut self.receiver).poll_next(cx) {
					// While have message
					Poll::Ready(Some((ref from, msg))) => {
						self.history.push(msg.clone());

						// Validate message.
						if let Some(hook) = &self.validator_hook {
							hook(&msg);
						}

						log::trace!("    msg {:?}", msg);
						// Broadcast to all peers including itself.
						for (to, sender) in &self.senders {
							if self.rule.lock().valid_route(from, to) {
								let _res = sender.unbounded_send(msg.clone());
							}
							// log::trace!("route: tx.isclosed? {}", sender.is_closed());
							// log::trace!("res: {:?}", res);
						}
					},
					Poll::Pending => return Poll::Pending,
					Poll::Ready(None) => return Poll::Ready(()),
				}
			}
		}

		/// Broadcast a message to all peers.
		pub fn send_message(&self, message: M) {
			// TODO: id: `0` is not used.
			let _ = self.raw_sender.unbounded_send((0, message));
		}
	}

	/// Network for a round.
	type RoundNetwork = BroadcastNetwork<SignedMessage<BlockNumber, Hash, Signature, Id>>;
	/// Global Network.
	type GlobalMessageNetwork = BroadcastNetwork<GlobalMessageIn<Hash, BlockNumber, Signature, Id>>;

	pub(crate) fn make_network() -> (Network, NetworkRouting) {
		let rule = Arc::new(Mutex::new(RoutingRule::new()));

		let rounds = Arc::new(Mutex::new(HashMap::new()));
		let global = Arc::new(Mutex::new(GlobalMessageNetwork::new(rule.clone())));
		(
			Network { rounds: rounds.clone(), global: global.clone(), rule: rule.clone() },
			NetworkRouting { rounds, global, rule },
		)
	}

	/// State of a voter.
	/// Used to pass to the [`Rule`] hook.
	#[derive(Default, Copy, Clone)]
	pub(crate) struct VoterState {
		pub(crate) last_finalized: BlockNumber,
		pub(crate) view_number: u64,
	}

	impl VoterState {
		pub fn new() -> Self {
			Self { last_finalized: 0, view_number: 0 }
		}
	}

	/// Rule type for routing table.
	type Rule = Box<dyn Send + Fn(&Id, &VoterState, &Id, &VoterState) -> bool>;

	/// Routing table.
	#[derive(Default)]
	pub(crate) struct RoutingRule {
		/// All peers in the network, same as VoterSet.
		nodes: Vec<Id>,
		/// Track all peers' state.
		state_tracker: HashMap<Id, VoterState>,
		/// Rule for routing.
		/// By default, all messages are broadcast.
		rules: Vec<Rule>,
	}

	impl RoutingRule {
		pub fn new() -> Self {
			Self { nodes: Vec::new(), state_tracker: HashMap::new(), rules: Vec::new() }
		}

		pub fn add_rule(&mut self, rule: Rule) {
			self.rules.push(rule);
		}

		/// Update peer's state.
		pub fn update_state(&mut self, id: Id, state: VoterState) {
			self.state_tracker.insert(id, state);
		}

		/// Check if a message is valid to route.
		pub fn valid_route(&mut self, from: &Id, to: &Id) -> bool {
			match (self.state_tracker.get(from), self.state_tracker.get(to)) {
				(Some(_), None) => {
					self.state_tracker.insert(to.clone(), VoterState::new());
				},
				(None, Some(_)) => {
					self.state_tracker.insert(from.clone(), VoterState::new());
				},
				(None, None) => {
					self.state_tracker.insert(from.clone(), VoterState::new());
					self.state_tracker.insert(to.clone(), VoterState::new());
				},
				_ => {},
			};

			let from_state = self.state_tracker.get(from).unwrap();
			let to_state = self.state_tracker.get(to).unwrap();
			self.rules.iter().map(|rule| rule(from, from_state, to, to_state)).all(|v| v)
		}

		/// A preset rule to isolate a peer from others.
		pub fn isolate(&mut self, node: Id) {
			let _isolate =
				move |from: &Id, _from_state: &VoterState, to: &Id, _to_state: &VoterState| {
					!(from == &node || to == &node)
				};
			self.add_rule(Box::new(_isolate));
		}

		/// A preset rule to isolate a peer from others after required block height.
		pub fn isolate_after(&mut self, node: Id, after: BlockNumber) {
			let _isolate_after =
				move |from: &Id, from_state: &VoterState, to: &Id, to_state: &VoterState| {
					!((from == &node && from_state.last_finalized >= after) ||
						(to == &node && to_state.last_finalized >= after))
				};
			self.add_rule(Box::new(_isolate_after));
		}
	}

	/// the network routing task.
	pub struct NetworkRouting {
		/// Key: view number, Value: RoundNetwork
		rounds: Arc<Mutex<HashMap<u64, RoundNetwork>>>,
		/// Global message network.
		global: Arc<Mutex<GlobalMessageNetwork>>,
		/// Routing rule.
		pub(crate) rule: Arc<Mutex<RoutingRule>>,
	}

	impl NetworkRouting {
		pub fn register_global_validator_hook(
			&mut self,
			hook: Box<
				dyn Fn(&GlobalMessageIn<Hash, BlockNumber, Signature, Id>) -> () + Sync + Send,
			>,
		) {
			self.global.lock().register_validator_hook(hook);
		}
	}

	impl Future for NetworkRouting {
		type Output = ();

		fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
			log::trace!("NetworkRouting::poll start.");
			let mut rounds = self.rounds.lock();
			// Retain all round that not finished
			rounds.retain(|view, round_network| {
				log::trace!("  view: {view} poll");
				let ret = match round_network.route(cx) {
					Poll::Ready(()) => {
						log::trace!(
							"    view: {view}, round_network.route: finished with history: {}",
							round_network.history.len()
						);

						false
					},
					Poll::Pending => true,
				};

				log::trace!("  view: {view} poll end");

				ret
			});

			let mut global = self.global.lock();
			let _ = global.route(cx);

			log::trace!("NetworkRouting::poll end.");

			// Nerver stop.
			Poll::Pending
		}
	}

	/// A test network. Instantiate this with `make_network`,
	#[derive(Clone)]
	pub struct Network {
		rounds: Arc<Mutex<HashMap<u64, RoundNetwork>>>,
		global: Arc<Mutex<GlobalMessageNetwork>>,
		rule: Arc<Mutex<RoutingRule>>,
	}

	impl Network {
		/// Initialize a round network.
		pub fn make_round_comms(
			&self,
			view_number: u64,
			node_id: Id,
		) -> (
			impl Stream<Item = Result<SignedMessage<BlockNumber, Hash, Signature, Id>, Error>>,
			impl Sink<Message<BlockNumber, Hash>, Error = Error>,
		) {
			log::trace!("make_round_comms, view_number: {}, node_id: {}", view_number, node_id);
			let mut rounds = self.rounds.lock();
			let round_comm = rounds
				.entry(view_number)
				.or_insert_with(|| RoundNetwork::new(self.rule.clone()))
				.add_node(node_id, move |message| SignedMessage {
					message,
					signature: node_id,
					id: node_id,
				});

			for (key, value) in rounds.iter() {
				log::trace!("  round_comms: {}, senders.len:{:?}", key, value.senders.len());
			}

			log::trace!("make_round_comms end");

			round_comm
		}

		/// Initialize the global network.
		pub fn make_global_comms(
			&self,
			id: Id,
		) -> (
			impl Stream<Item = Result<GlobalMessageIn<Hash, BlockNumber, Signature, Id>, Error>>,
			impl Sink<GlobalMessageOut<Hash, BlockNumber, Signature, Id>, Error = Error>,
		) {
			log::trace!("make_global_comms");
			let mut global = self.global.lock();
			let f: fn(GlobalMessageOut<_, _, _, _>) -> GlobalMessageIn<_, _, _, _> = |msg| match msg
			{
				GlobalMessageOut::Commit(view, commit) =>
					GlobalMessageIn::Commit(view, commit.into(), Callback::Blank),

				GlobalMessageOut::ViewChange(view_change) =>
					GlobalMessageIn::ViewChange(view_change),
				GlobalMessageOut::Empty => GlobalMessageIn::Empty,
			};
			let global_comm = global.add_node(id, f);

			global_comm
		}

		/// Send a message to all nodes.
		pub fn send_message(&self, message: GlobalMessageIn<Hash, BlockNumber, Signature, Id>) {
			self.global.lock().send_message(message);
		}
	}

	/// A implementation of `Environment`.
	pub struct DummyEnvironment {
		local_id: Id,
		network: Network,
		listeners: Mutex<Vec<UnboundedSender<(Hash, BlockNumber)>>>,
		chain: Mutex<DummyChain>,
	}

	impl DummyEnvironment {
		pub fn new(network: Network, local_id: Id) -> Self {
			DummyEnvironment {
				network,
				local_id,
				chain: Mutex::new(DummyChain::new()),
				listeners: Mutex::new(Vec::new()),
			}
		}

		pub fn with_chain<F, U>(&self, f: F) -> U
		where
			F: FnOnce(&mut DummyChain) -> U,
		{
			let mut chain = self.chain.lock();
			f(&mut *chain)
		}

		pub fn finalized_stream(&self) -> UnboundedReceiver<(Hash, BlockNumber)> {
			let (tx, rx) = mpsc::unbounded();
			self.listeners.lock().push(tx);
			rx
		}

		pub async fn run(&self) {
			Delay::new(Duration::from_millis(1000)).await;
			log::trace!("hello");
			Delay::new(Duration::from_millis(1000)).await;
			log::trace!("world");
			Delay::new(Duration::from_millis(1000)).await;
		}
	}

	impl Environment for DummyEnvironment {
		type Timer = Box<dyn Future<Output = Result<(), Error>> + Unpin + Send>;
		type Id = Id;
		type Signature = Signature;
		type BestChain = Box<
			dyn Future<Output = Result<Option<(Self::Number, Self::Hash)>, Error>> + Unpin + Send,
		>;
		type In = Box<
			dyn Stream<Item = Result<SignedMessage<BlockNumber, Hash, Signature, Id>, Error>>
				+ Unpin
				+ Send,
		>;
		type Out = Pin<Box<dyn Sink<Message<BlockNumber, Hash>, Error = Error> + Send>>;
		type Error = Error;
		type Hash = Hash;
		type Number = BlockNumber;

		fn voter_data(&self) -> VoterData<Self::Id> {
			VoterData { local_id: self.local_id }
		}

		fn round_data(&self, view: u64) -> RoundData<Self::Id, Self::In, Self::Out> {
			log::trace!("{:?} round_data view: {}", self.local_id, view);
			const GOSSIP_DURATION: Duration = Duration::from_millis(500);

			let (incomming, outgoing) = self.network.make_round_comms(view, self.local_id);
			RoundData {
				voter_id: self.local_id,
				// prevote_timer: Box::new(Delay::new(GOSSIP_DURATION).map(Ok)),
				incoming: Box::new(incomming),
				outgoing: Box::pin(outgoing),
			}
		}

		fn finalize_block(
			&self,
			view: u64,
			hash: Self::Hash,
			number: Self::Number,
			_f_commit: FinalizedCommit<Self::Number, Self::Hash, Self::Signature, Self::Id>,
		) -> Result<(), Self::Error> {
			log::trace!("{:?} finalize_block", self.local_id);
			self.chain.lock().finalize_block(hash);
			self.listeners.lock().retain(|s| s.unbounded_send((hash, number)).is_ok());

			// Update Network RoutingRule's state.
			self.network.rule.lock().update_state(
				self.local_id,
				VoterState { view_number: view, last_finalized: number },
			);

			Ok(())
		}

		fn preprepare(&self, _view: u64, _block: Self::Hash) -> Self::BestChain {
			Box::new(futures::future::ok(Some(self.with_chain(|chain| {
				chain.next_to_be_finalized().unwrap_or_else(|_| chain.last_finalized())
			}))))
		}

		fn complete_f_commit(
			&self,
			_view: u64,
			_state: crate::leader::State<Self::Number, Self::Hash>,
			_base: (Self::Number, Self::Hash),
			_f_commit: FinalizedCommit<Self::Number, Self::Hash, Self::Signature, Self::Id>,
		) -> Result<(), Self::Error> {
			Ok(())
		}
	}
}

#[cfg(test)]
mod test {
	use std::{sync::Arc, time::Duration};

	use futures::{executor::LocalPool, future, task::SpawnExt, SinkExt, StreamExt};
	use futures_timer::Delay;
	use parking_lot::Mutex;

	use super::environment::make_network;
	use crate::leader::voter::{GlobalMessageIn, GlobalMessageOut};

	#[test]
	fn test_validator_hook() {
		let (network, mut routing_network) = make_network();
		let nodes = 0..2;

		let count = Arc::new(Mutex::new(0));

		let count_clone = count.clone();

		routing_network.register_global_validator_hook(Box::new(move |m| {
			let mut count = count.lock();
			*count += 1;
			assert_eq!(m, &GlobalMessageIn::Empty);
		}));
		assert_eq!(*count_clone.lock(), 0);

		let (nodes, mut nodes_in): (Vec<_>, Vec<_>) = nodes
			.into_iter()
			.map(|id| {
				let (outgo, income) = network.make_global_comms(id);

				let outgo = outgo.take(1).for_each(|msg| {
					assert_eq!(msg.unwrap(), GlobalMessageIn::Empty);
					future::ready(())
				});

				(outgo, income)
			})
			.unzip();

		let mut pool = LocalPool::new();
		pool.spawner().spawn(routing_network).unwrap();

		pool.spawner()
			.spawn(async move {
				let node = nodes_in.get_mut(0).unwrap();
				node.send(GlobalMessageOut::Empty).await.unwrap();
			})
			.unwrap();

		pool.run_until(futures::future::join_all(nodes.into_iter()));
		assert_eq!(*count_clone.lock(), 1);
	}

	#[test]
	// #[ntest::timeout(1000)]
	fn routing_rule_allow_all_in_default() {
		let (network, routing_network) = make_network();
		let nodes = 0..2;

		let (nodes, mut nodes_in): (Vec<_>, Vec<_>) = nodes
			.into_iter()
			.map(|id| {
				let (outgo, income) = network.make_global_comms(id);

				let outgo = outgo.take(1).for_each(|msg| {
					assert_eq!(msg.unwrap(), GlobalMessageIn::Empty);
					future::ready(())
				});

				(outgo, income)
			})
			.unzip();

		let mut pool = LocalPool::new();
		pool.spawner().spawn(routing_network).unwrap();

		pool.spawner()
			.spawn(async move {
				let node = nodes_in.get_mut(0).unwrap();
				node.send(GlobalMessageOut::Empty).await.unwrap();
			})
			.unwrap();

		pool.run_until(futures::future::join_all(nodes.into_iter()));
	}

	#[test]
	fn disable_routing() {
		let (network, routing_network) = make_network();
		let nodes = 0..2;

		routing_network.rule.lock().isolate(0);
		routing_network.rule.lock().isolate(1);
		routing_network.rule.lock().isolate(2);

		let (nodes, mut nodes_in): (Vec<_>, Vec<_>) = nodes
			.into_iter()
			.map(|id| {
				let (outgo, income) = network.make_global_comms(id);

				let outgo = outgo.take(1).for_each(|msg| {
					assert!(false, "should not be routed, result: {:?}", msg);
					future::ready(())
				});

				let outgo = async move {
					let timeout = Delay::new(Duration::from_millis(1000));

					futures::future::select(outgo, timeout).await;
				};

				(outgo, income)
			})
			.unzip();

		let mut pool = LocalPool::new();
		pool.spawner().spawn(routing_network).unwrap();

		pool.spawner()
			.spawn(async move {
				let node = nodes_in.get_mut(0).unwrap();
				node.send(GlobalMessageOut::Empty).await.unwrap();
			})
			.unwrap();

		pool.run_until(futures::future::join_all(nodes.into_iter()));
	}
}
