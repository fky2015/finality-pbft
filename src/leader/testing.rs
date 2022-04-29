type Id = u64;
type Signature = u64;
type Hash = &'static str;
type BlockNumber = u64;

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

	#[derive(Debug)]
	pub struct DummyChain {
		inner: BTreeMap<Hash, BlockRecord>,
		finalized: (Hash, BlockNumber),
	}

	impl DummyChain {
		pub fn new() -> Self {
			let mut inner = BTreeMap::new();
			inner.insert(GENESIS_HASH, BlockRecord { number: 1, parent: NULL_HASH });
			DummyChain { inner, finalized: (GENESIS_HASH, 1) }
		}

		pub fn push_blocks(&mut self, mut parent: Hash, blocks: &[Hash]) {
			if blocks.is_empty() {
				return;
			}

			for i in blocks {
				self.push_block(parent, i);
				parent = i
			}
		}

		pub fn push_block(&mut self, parent: Hash, block: Hash) {
			let block_number = self.inner.get(parent).unwrap().number + 1;
			self.inner.insert(block, BlockRecord { number: block_number, parent });
		}

		pub fn last_finalized(&self) -> (Hash, BlockNumber) {
			self.finalized
		}

		/// Get block after the last finalized block
		pub fn next_to_be_finalized(&self) -> Result<(Hash, BlockNumber), ()> {
			for (hash, record) in self.inner.iter().rev() {
				if record.number == self.finalized.1 + 1 {
					return Ok((hash.clone(), record.number));
				}
			}

			Err(())
		}

		pub fn finalize_block(&mut self, block: Hash) -> bool {
			#[cfg(feature = "std")]
			log::trace!("finalize_block: {:?}", block);
			if let Some(b) = self.inner.get(&block) {
				#[cfg(feature = "std")]
				log::trace!("finalize block: {:?}", b);
				if self.inner.get(&b.parent).map(|p| p.number + 1 == b.number).unwrap_or(false) {
					self.finalized = (block, b.number);
					#[cfg(feature = "std")]
					log::trace!("new finalized = {:?}", self.finalized);
					return true;
				} else {
					return false;
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

	use crate::leader::{
		testing::chain::DummyChain,
		voter::{
			communicate::{RoundData, VoterData},
			Callback, Environment, GlobalMessageIn, GlobalMessageOut,
		},
		Error, FinalizedCommit, Message, SignedMessage,
	};

	use super::*;
	use futures::{
		channel::mpsc::{self, UnboundedReceiver, UnboundedSender},
		Future, FutureExt, Sink, SinkExt, Stream, StreamExt,
	};
	use futures_timer::Delay;
	use parking_lot::Mutex;

	/// p2p network data for a round.
	///
	/// Every node can send `Message` to the network, then it will be
	/// wrapped in `SignedMessage` and broadcast to all other nodes.
	struct BroadcastNetwork<M> {
		receiver: UnboundedReceiver<M>,
		raw_sender: UnboundedSender<M>,
		/// Peer's handle to send messages to.
		senders: Vec<UnboundedSender<M>>,
		history: Vec<M>,
	}

	impl<M: Clone + std::fmt::Debug> BroadcastNetwork<M> {
		fn new() -> Self {
			let (tx, rx) = mpsc::unbounded();
			BroadcastNetwork {
				receiver: rx,
				raw_sender: tx,
				senders: Vec::new(),
				history: Vec::new(),
			}
		}

		/// Add a node to the network for a round.
		fn add_node<N, F: Fn(N) -> M>(
			&mut self,
			f: F,
		) -> (impl Stream<Item = Result<M, Error>>, impl Sink<N, Error = Error>) {
			log::trace!("BroadcastNetwork::add_node");
			// Channel from Network to the new node.
			let (tx, rx) = mpsc::unbounded();
			let messages_out = self
				.raw_sender
				.clone()
				.sink_map_err(|e| panic!("Error sending message: {:?}", e))
				.with(move |message| std::future::ready(Ok(f(message))));

			// get history to the node.
			// for prior_message in self.history.iter().cloned() {
			// 	let _ = tx.unbounded_send(prior_message);
			// }

			// log::trace!("add_node: tx.isclosed? {}", tx.is_closed());
			self.senders.push(tx);

			log::trace!("BroadcastNetwork::add_node end.");
			(rx.map(Ok), messages_out)
		}

		// Do routing work
		fn route(&mut self, cx: &mut Context) -> Poll<()> {
			loop {
				// Receive item from receiver
				match Pin::new(&mut self.receiver).poll_next(cx) {
					// While have message
					Poll::Ready(Some(msg)) => {
						self.history.push(msg.clone());

						log::trace!("    msg {:?}", msg);
						// broadcast to all peer (TODO: including itself?)
						for sender in &self.senders {
							// log::trace!("route: tx.isclosed? {}", sender.is_closed());
							let _res = sender.unbounded_send(msg.clone());
							// log::trace!("res: {:?}", res);
						}
					},
					Poll::Pending => return Poll::Pending,
					Poll::Ready(None) => return Poll::Ready(()),
				}
			}
		}
	}

	struct CollectorNetwork<M> {
		receiver: UnboundedReceiver<M>,
		raw_sender: UnboundedSender<M>,
		history: Vec<M>,
	}

	impl<M: Clone> CollectorNetwork<M> {
		fn new() -> Self {
			let (tx, rx) = mpsc::unbounded();
			CollectorNetwork { receiver: rx, raw_sender: tx, history: Vec::new() }
		}

		/// Add a node to network
		fn add_node<N, F: Fn(N) -> M>(&mut self, f: F) -> impl Sink<N, Error = Error> {
			let messages_out = self
				.raw_sender
				.clone()
				.sink_map_err(|e| panic!("Error sending message: {:?}", e))
				.with(move |message| std::future::ready(Ok(f(message))));

			messages_out
		}

		fn route(&mut self, cx: &mut Context) -> Poll<()> {
			loop {
				// Receive item from receiver
				match Pin::new(&mut self.receiver).poll_next(cx) {
					// While have message
					Poll::Ready(Some(msg)) => {
						self.history.push(msg.clone());
					},
					Poll::Pending => return Poll::Pending,
					Poll::Ready(None) => return Poll::Ready(()),
				}
			}
		}
	}

	// type LogNetwork = CollectorNetwork<report::Log<&'static str, Id>>;
	type RoundNetwork = BroadcastNetwork<SignedMessage<BlockNumber, Hash, Signature, Id>>;
	type GlobalMessageNetwork = BroadcastNetwork<GlobalMessageIn<Hash, BlockNumber, Signature, Id>>;

	pub(crate) fn make_network() -> (Network, NetworkRouting) {
		let rounds = Arc::new(Mutex::new(HashMap::new()));
		let global = Arc::new(Mutex::new(GlobalMessageNetwork::new()));
		// let log_collector = Arc::new(Mutex::new(CollectorNetwork::new()));
		(
			Network { rounds: rounds.clone(), global: global.clone() },
			NetworkRouting { rounds, global },
		)
	}

	/// the network routing task.
	pub struct NetworkRouting {
		/// Key: view/round number, Value: RoundNetwork
		rounds: Arc<Mutex<HashMap<u64, RoundNetwork>>>,
		global: Arc<Mutex<GlobalMessageNetwork>>,
		// Log collector
		// log_collector: Arc<Mutex<LogNetwork>>,
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
		/// Network for the round.
		rounds: Arc<Mutex<HashMap<u64, RoundNetwork>>>,
		global: Arc<Mutex<GlobalMessageNetwork>>,
		// Log collector
		// log_collector: Arc<Mutex<LogNetwork>>,
	}

	impl Network {
		pub fn make_round_comms(
			&self,
			view_number: u64,
			node_id: Id,
		) -> (
			impl Stream<Item = Result<SignedMessage<BlockNumber, Hash, Signature, Id>, Error>>,
			impl Sink<Message<BlockNumber, Hash>, Error = Error>,
		) {
			log::trace!("make_round_comms, view_number: {}, node_id: {}", view_number, node_id);
			// let log_sender = self
			// 	.log_collector
			// 	.lock()
			// 	.add_node(move |log| report::Log { id: node_id.clone(), state: log });
			let mut rounds = self.rounds.lock();
			let round_comm = rounds.entry(view_number).or_insert_with(RoundNetwork::new).add_node(
				move |message| SignedMessage { message, signature: node_id, id: node_id },
			);

			for (key, value) in rounds.iter() {
				log::trace!("  round_comms: {}, senders.len:{:?}", key, value.senders.len());
			}

			log::trace!("make_round_comms end");

			round_comm
		}

		pub fn make_global_comms(
			&self,
		) -> (
			impl Stream<Item = Result<GlobalMessageIn<Hash, BlockNumber, Signature, Id>, Error>>,
			impl Sink<GlobalMessageOut<Hash, BlockNumber, Signature, Id>, Error = Error>,
		) {
			log::trace!("make_global_comms");
			let mut global = self.global.lock();
			let f: fn(GlobalMessageOut<_, _, _, _>) -> GlobalMessageIn<_, _, _, _> = |msg| match msg
			{
				GlobalMessageOut::Commit(view, commit) => {
					GlobalMessageIn::Commit(view, commit.into(), Callback::Blank)
				},

				GlobalMessageOut::ViewChange(view_change) => {
					GlobalMessageIn::ViewChange(view_change)
				},
				GlobalMessageOut::Empty => GlobalMessageIn::Empty,
			};
			let global_comm = global.add_node(f);

			global_comm
		}
	}

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
			dyn Future<Output = Result<Option<(Self::Hash, Self::Number)>, Error>> + Unpin + Send,
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
			f_commit: FinalizedCommit<Self::Number, Self::Hash, Self::Signature, Self::Id>,
		) -> Result<(), Self::Error> {
			log::trace!("{:?} finalize_block", self.local_id);
			self.chain.lock().finalize_block(hash);
			for i in self.listeners.lock().iter() {
				i.unbounded_send((hash, number)).unwrap();
			}

			Ok(())
		}

		fn preprepare(&self, _view: u64) -> Self::BestChain {
			Box::new(futures::future::ok(Some(self.with_chain(|chain| {
				log::info!(
					"chain: {:?}, last_finalized: {:?}, next_to_be_finalized: {:?}",
					chain,
					chain.last_finalized(),
					chain.next_to_be_finalized()
				);
				chain.next_to_be_finalized().unwrap_or_else(|_| chain.last_finalized())
			}))))
		}
	}
}
