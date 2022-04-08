//! pbft finality gadget.
//!
#![allow(missing_docs)]
#![allow(dead_code)]

use env::Environment;
use futures::{FutureExt, Sink, SinkExt, Stream, TryStreamExt};
use futures_timer::Delay;
#[cfg(feature = "derive-codec")]
use parity_scale_codec::{Decode, Encode};
use parking_lot::Mutex;
#[cfg(feature = "derive-codec")]
use scale_info::TypeInfo;
use std::{collections::HashMap, sync::Arc, time::Duration};

/// Error for PBFT consensus.
#[derive(Debug, Clone)]
pub enum Error {
	// incase of primary failure, need to view change.
	// NoPrePrepare,
	/// No Primary message was received.
	PrimaryFailure,
}

impl std::fmt::Display for Error {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		match *self {
			// Error::NoPrePrepare => {
			// 	write!(f, "No preprepare message, may need to initiate view change.")
			// },
			Error::PrimaryFailure => {
				write!(f, "No primary message, may need to initiate view change.")
			},
		}
	}
}

pub mod view_round {

	/// State of the round.
	#[derive(PartialEq, Clone)]
	#[cfg_attr(any(feature = "std", test), derive(Debug))]
	#[cfg_attr(feature = "derive-codec", derive(Encode, Decode, scale_info::TypeInfo))]
	pub struct State<H, N> {
		/// The finalized block.
		pub finalized: Option<(H, N)>,
		/// Whether the view is completable.
		pub completable: bool,
	}

	impl<H: Clone, N: Clone> State<H, N> {
		/// Genesis state.
		pub fn genesis(genesis: (H, N)) -> Self {
			State {
				finalized: Some(genesis.clone()),
				completable: true,
			}
		}
	}
}

impl std::error::Error for Error {}

pub(crate) mod helper {
	// TODO: eliminate this
	pub fn supermajority(n: usize, valid_number: usize) -> bool {
		n > 3 * (n - valid_number)
	}
}

/// Communication helper data
pub mod communicate {
	/// Data necessary to create a voter.
	pub struct VoterData<ID> {
		/// Local voter id.
		pub local_id: ID,
	}

	/// Data necessary to participate in a round.
	pub struct RoundData<ID, Timer, Input, Output> {
		/// Local voter id
		pub voter_id: ID,
		/// Timer before prevotes can be cast. This should be Start + 2T
		/// where T is the gossip time estimate.
		pub prevote_timer: Timer,
		/// Timer before precommits can be cast. This should be Start + 4T
		// pub precommit_timer: Timer,
		/// Incoming messages.
		pub incoming: Input,
		/// Outgoing messages.
		pub outgoing: Output,
		// Output state log
		// pub log_sender: LogOutput,
	}
}

pub mod report {
	use super::CurrentState;
	use std::collections::HashSet;

	#[derive(Clone, Debug)]
	pub struct ViewState<Hash, ID> {
		pub state: CurrentState,
		pub total_number: usize,
		pub threshold: usize,
		pub preprepare_hash: Option<Hash>,
		pub prepare_count: usize,
		/// The identities of nodes that have cast prepare so far.
		pub prepare_ids: HashSet<ID>,
		pub commit_count: usize,
		pub commit_ids: HashSet<ID>,
	}

	#[derive(Clone, Debug)]
	pub struct Log<Hash, ID> {
		pub id: ID,
		pub state: ViewState<Hash, ID>,
	}
}

/// Environment for consensus.
///
/// Including network, storage and other info (that cannot get directly via consensus algo).
pub mod env {
	use std::collections::HashMap;

	use futures::{Future, Sink, Stream};
	use parking_lot::Mutex;

	use crate::BlockNumberOps;

	use super::{Message, SignedMessage};

	/// Necessary environment for a voter.
	///
	/// This encapsulates the database and networking layers of the chain.
	pub trait Environment {
		/// Associated timer type for the environment. See also [`Self::round_data`] and
		/// [`Self::round_commit_timer`].
		type Timer: Future<Output = Result<(), Self::Error>> + Unpin;
		/// The associated ID for the Environment.
		type ID: Clone + Eq + std::hash::Hash + Ord + std::fmt::Debug;
		/// The associated Signature type for the Environment.
		type Signature: Eq + Clone + core::fmt::Debug;
		/// The input stream used to communicate with the outside world.
		type In: Stream<
				Item = Result<
					SignedMessage<Self::Number, Self::Hash, Self::Signature, Self::ID>,
					Self::Error,
				>,
			> + Unpin;
		/// The output stream used to communicate with the outside world.
		type Out: Sink<Message<Self::Number, Self::Hash>, Error = Self::Error> + Unpin;
		/// The associated Error type.
		type Error: From<super::Error> + ::std::error::Error + Clone;
		/// Hash type used in blockchain or digest.
		type Hash: Eq + Clone + core::fmt::Debug;
		/// The block number type.
		type Number: BlockNumberOps;

		/// Get Voter data.
		fn voter_data(&self) -> super::communicate::VoterData<Self::ID>;

		/// Get round data.
		fn round_data(
			&self,
			view: u64,
		) -> super::communicate::RoundData<Self::ID, Self::Timer, Self::In, Self::Out>;

		/// preprepare
		fn preprepare(&self, view: u64) -> Option<(Self::Hash, Self::Number)>;

		/// Finalize a block.
		// TODO: maybe async?
		fn finalize_block(
			&self,
			hash: Self::Hash,
			number: Self::Number,
			// commit: Message::Commit,
		) -> bool;
	}

	pub struct Logger<E>
	where
		E: Environment,
	{
		storage: Mutex<HashMap<E::ID, Vec<super::report::ViewState<E::Hash, E::ID>>>>,
	}

	impl<E> Logger<E>
	where
		E: Environment,
	{
		pub fn push_view_state(
			&mut self,
			id: E::ID,
			state: super::report::ViewState<E::Hash, E::ID>,
		) {
			self.storage.lock().entry(id).or_insert(Vec::new()).push(state);
		}
	}
}

/// V: view number
/// N: sequence number
/// D: Digest of the block, that is header
#[derive(Clone)]
#[cfg_attr(any(feature = "std", test), derive(Debug))]
#[cfg_attr(feature = "derive-codec", derive(Encode, Decode, TypeInfo))]
pub enum Message<N, D> {
	/// A message to be sent to the network by primary alive but not yet haprimary alive but not
	/// yet have a new block to make consensus.
	EmptyPrePrepare,
	/// multicast (except from the primary) <view, number, Digest(m)>
	Prepare { view: u64, seq_number: N, digest: D },
	/// multicast <view, number>
	Commit { view: u64, seq_number: N },
}

#[derive(Clone)]
#[cfg_attr(any(feature = "std", test), derive(Debug))]
#[cfg_attr(feature = "derive-codec", derive(Encode, Decode, TypeInfo))]
pub enum GlobalMessage<ID> {
	/// multicast <view + 1, latest stable checkpoint, C: a set of pairs with the sequence number
	/// and digest of each checkpoint, P, Q, i>
	ViewChange {
		new_view: u64,
		id: ID,
		// latest_checkpoint: u64,
		// checkpoints: Vec<(u64, D)>,
	},
	// reply <view + 1, i, j, digest>
	// ViewChangeAck {
	// 	new_view: u64,
	// 	// i: u64,
	// 	// j: u64,
	// 	// digest: D,
	// },
	// multicast (from the primary) <view + 1, V, X>
	// NewView {
	// 	new_view: u64,
	// 	// V: u64,
	// 	// X: D,
	// },
	// multicast <number, digest, i>
	// CheckPoint {},
	/// NOTE: This is a hack to make the consensus work.
	Empty,
}

#[derive(Clone, Debug)]
pub struct SignedMessage<N, H, Signature, ID> {
	from: ID,
	message: Message<N, H>,
	signature: Signature,
}

#[derive(PartialEq, Clone, Debug)]
pub enum CurrentState {
	// Initial state.
	PrePrepare,
	Prepare,
	Commit,
	// ChangeView,
	// ViewChangeAck,
	// NewView,
}

// trait MessageStorage<H> {
// 	fn save_message(&mut self, hash: H, message: Message);
// 	fn count_prepare(&self, hash: H) -> bool;
// 	fn count_commit(&self, hash: H) -> bool;
// }

/// similar to: [`round::Round`]
#[derive(Debug)]
struct Storage<N, H, ID> {
	preprepare_hash: Option<H>,
	preprepare: HashMap<ID, ()>,
	prepare: HashMap<ID, Message<N, H>>,
	commit: HashMap<ID, Message<N, H>>,
}

impl<N, H, ID> Storage<N, H, ID>
where
	ID: Clone + Eq + std::hash::Hash + Ord + std::fmt::Debug,
	H: std::fmt::Debug,
	N: std::fmt::Debug,
{
	fn new() -> Self {
		Self {
			preprepare_hash: None,
			preprepare: Default::default(),
			prepare: Default::default(),
			commit: Default::default(),
		}
	}

	fn contains_key(&self, key: &ID) -> bool {
		self.preprepare.contains_key(key)
			|| self.prepare.contains_key(key)
			|| self.commit.contains_key(key)
	}
}

impl<N, H, ID: Eq + std::hash::Hash> Storage<N, H, ID>
where
	ID: std::fmt::Debug,
	H: std::fmt::Debug,
	N: std::fmt::Debug,
{
	fn save_message(&mut self, from: ID, message: Message<N, H>) {
		log::trace!("insert message to Storage, from: {:?}", from);
		match message {
			msg @ Message::Prepare { .. } => {
				log::trace!("insert message to Prepare, msg: {:?}", msg);
				self.prepare.insert(from, msg);
			},
			msg @ Message::Commit { .. } => {
				log::trace!("insert message to Commit, msg: {:?}", msg);
				self.commit.insert(from, msg);
			},
			Message::EmptyPrePrepare => {
				log::trace!("insert message to preprepare, msg: EmptyPrePrepare");
				self.preprepare.insert(from, ());
			},
		}
	}

	fn print_log(&self) {
		log::trace!("pre-prepare: {:?}", self.preprepare_hash);
		for i in self.prepare.iter() {
			log::trace!("  {:?}", i);
		}
		for i in self.commit.iter() {
			log::trace!("  {:?}", i);
		}
		log::trace!("=== end ===")
	}

	fn count_prepare(&self) -> usize {
		self.prepare.len()
	}

	fn count_commit(&self) -> usize {
		self.commit.len()
	}
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct VoterSet<ID: Eq + Ord> {
	voters: Vec<ID>,
	/// The required number threshould for supermajority.
	/// Normally, it's > 2/3.
	threshould: usize,
}

impl<ID: Eq + Ord + Clone> VoterSet<ID> {
	pub fn new(voters: Vec<ID>) -> Self {
		let len = voters.len() / 3 + 1;
		Self { voters, threshould: len }
	}

	pub fn add(&mut self, id: ID) {
		self.voters.push(id);
	}

	pub fn remove(&mut self, id: &ID) {
		self.voters.retain(|x| x != id);
	}

	pub fn is_empty(&self) -> bool {
		self.voters.is_empty()
	}

	pub fn is_full(&self) -> bool {
		self.voters.len() >= self.threshould
	}

	pub fn is_member(&self, id: &ID) -> bool {
		self.voters.contains(id)
	}

	pub fn len(&self) -> usize {
		self.voters.len()
	}

	pub fn threshould(&self) -> usize {
		self.threshould
	}

	pub fn voters(&self) -> &[ID] {
		&self.voters
	}

	pub fn get_primary(&self, view: u64) -> ID {
		self.voters.get(view as usize % self.voters.len()).cloned().unwrap()
	}
}

pub struct Voter<E: Environment, GlobalIn, GlobalOut> {
	local_id: E::ID,
	env: Arc<E>,
	voters: VoterSet<E::ID>,
	global_incoming: GlobalIn,
	global_outgoing: GlobalOut,
	// TODO: consider wrap this around sth.
	inner: Option<(ViewRound<E>, E::In)>,
	last_finalized_number: E::Number,
	current_view_number: u64,
}

// impl<
// 		E: Environment, // GlobalIn, GlobalOut
// 	> Unpin for Voter<E>
// {
// }

impl<E: Environment, GlobalIn, GlobalOut> Voter<E, GlobalIn, GlobalOut>
where
	GlobalIn: Stream<Item = Result<GlobalMessage<E::ID>, E::Error>> + Unpin,
	GlobalOut: Sink<GlobalMessage<E::ID>, Error = E::Error> + Unpin,
{
	pub fn new(
		env: Arc<E>,
		voters: VoterSet<E::ID>,
		global_comms: (GlobalIn, GlobalOut),
		current_view_number: u64,
		last_round_base: (E::Hash, E::Number),
	) -> Self {
		let voter_data = env.voter_data();
		Self {
			local_id: voter_data.local_id,
			env: env.clone(),
			voters,
			global_incoming: global_comms.0,
			global_outgoing: global_comms.1,
			last_finalized_number: last_round_base.1,
			inner: None,
			current_view_number,
		}
	}

	pub fn new_view_round(&self) -> (ViewRound<E>, E::In) {
		let voter_set = self.voters.clone();
		let round_data = self.env.round_data(self.current_view_number);
		(
			ViewRound::new(
				self.current_view_number,
				self.last_finalized_number,
				voter_set,
				self.env.clone(),
				round_data.voter_id,
				round_data.outgoing,
			),
			round_data.incoming,
		)
	}

	pub async fn process_voting_round(
		inner_incoming: &mut E::In,
		inner: &mut ViewRound<E>,
	) -> Result<(), E::Error> {
		// FIXME: optimize
		log::trace!("{:?} process_voting_round {:?}", inner.id, inner.view);
		let a = ViewRound::<E>::process_incoming(inner_incoming, inner.message_log.clone()).fuse();
		let b = inner.progress().fuse();

		futures::pin_mut!(a, b);

		futures::select! {
			a = a => {
				log::trace!("a ready {:?}", a);
				a
			}
			b = b => {
				log::trace!("b ready {:?}", b);
				b
			}
		}
	}

	pub async fn change_view(
		current_view: u64,
		id: E::ID,
		global_outgoing: &mut GlobalOut,
		global_incoming: &mut GlobalIn,
		voters: &VoterSet<E::ID>,
	) -> Result<u64, E::Error> {
		const DELAY_VIEW_CHANGE_WAIT: u64 = 1000;
		let mut new_view = current_view + 1;
		loop {
			log::info!("id: {:?}, start view_change: {new_view}", id);
			// create new view change message
			let view_change = GlobalMessage::ViewChange { new_view, id: id.clone() };
			let mut log = HashMap::<E::ID, GlobalMessage<E::ID>>::new();

			global_outgoing.send(view_change.clone()).await?;

			let mut timeout = Delay::new(Duration::from_millis(DELAY_VIEW_CHANGE_WAIT)).fuse();

			loop {
				// Stream.next with timeout
				futures::select! {
					_ = timeout => {
						break;
					},
					msg = global_incoming.try_next().fuse() => {
						let msg = msg?.unwrap();
						if let GlobalMessage::ViewChange { new_view : new_view_received, id} = msg.clone() {
							if new_view == new_view_received {
								log::trace!("{:?} save valid viewChange {:?}", id, msg);
								log.insert(id, msg);
							}
						}
					}
				}
			}

			if log.len() >= voters.threshould() {
				log::info!("id: {:?}, successfully view_change: {new_view}", id);
				return Ok(new_view);
			} else {
				// if primary in new view also send view change message, keep view number unchanged
				let primary = voters.get_primary(new_view);
				if log.contains_key(&primary) {
				} else {
					new_view += 1;
				}
			}
		}
	}

	pub async fn run(&mut self) {
		log::trace!("{:?} Voter::run", self.local_id);

		loop {
			let (mut view_round, mut incoming) = self.new_view_round();

			// NOTE: this is a hack to make sure new network is activated, by sending a empty
			// mesage to the network.
			//
			// FIXME: This message could be eliminated by implmenting following step:
			// a) always create next-view network aggressively, even in the case of a view change.
			// a.1) In detail, the network should be created when the view number changed.
			let _ = self.global_outgoing.send(GlobalMessage::Empty).await;

			// run both global_incoming and view_round
			// wait one of them return
			let result = Voter::<E, GlobalIn, GlobalOut>::process_voting_round(
				&mut incoming,
				&mut view_round,
			)
			.await;

			// restart new voting round.
			log::trace!("{:?} current voting round finish: {:?}", self.local_id, result);
			if let Err(e) = result.clone() {
				log::warn!("Error throw by ViewRound: {:?}", e);
				// TODO: may need to change view
				let new_view = Voter::<E, GlobalIn, GlobalOut>::change_view(
					self.current_view_number,
					self.local_id.clone(),
					&mut self.global_outgoing,
					&mut self.global_incoming,
					&self.voters,
				)
				.await
				.unwrap();
				self.current_view_number = new_view;
			}
		}
	}

	// pub async fn process_current_consensus(&self) {}
}

/// Logic for a voter on a specific round.
/// Similar to [`voting_round::VotingRound`]
pub struct ViewRound<E: Environment> {
	env: Arc<E>,
	voter_set: VoterSet<E::ID>,
	current_state: CurrentState,
	message_log: Arc<Mutex<Storage<E::Number, E::Hash, E::ID>>>,
	view: u64,
	current_height: E::Number,
	id: E::ID,
	// incoming: E::In,
	outgoing: E::Out,
}

impl<E> ViewRound<E>
where
	E: Environment,
{
	// TODO: maybe delete some fields?
	pub fn new(
		view: u64,
		current_height: E::Number,
		// preprepare_hash: H,
		voter_set: VoterSet<E::ID>,
		env: Arc<E>,
		voter_id: E::ID,
		outgoing: E::Out,
	) -> Self {
		ViewRound {
			env,
			view,
			voter_set,
			current_state: CurrentState::PrePrepare,
			message_log: Arc::new(Mutex::new(Storage::new())),
			current_height,
			id: voter_id,
			// incoming: round_data.incoming,
			// TODO: why bufferd
			outgoing,
		}
	}

	fn change_state(&mut self, state: CurrentState) {
		// TODO: push log
		self.current_state = state;
	}

	async fn process_incoming(
		incoming: &mut E::In,
		log: Arc<Mutex<Storage<E::Number, E::Hash, E::ID>>>,
	) -> Result<(), E::Error> {
		// FIXME: optimize
		log::trace!("start of process_incoming");
		while let Some(msg) = incoming.try_next().await? {
			log::trace!("process_incoming: {:?}", msg);
			let SignedMessage { message, .. } = msg;
			log.lock().save_message(msg.from, message);
		}

		log::trace!("end of process_incoming");
		Ok(())
	}

	fn is_primary(&self) -> bool {
		self.voter_set.get_primary(self.view) == self.id
	}

	fn validate_preprepare(&mut self) -> bool {
		log::trace!("{:?} start validate_preprepare", self.id);
		if self.is_primary() {
			self.change_state(CurrentState::Prepare);
		} else {
			// TODO: A backup i accepts the PRE_PREPARE message provoided it has not accepts a PRE-PREPARE
			// for view v and sequence number n containing a different digeset.
			// ask environment for new block
			self.change_state(CurrentState::Prepare);
		}
		true
	}

	async fn preprepare(&mut self) -> Result<(), E::Error> {
		const DELAY_PRE_PREPARE_MILLI: u64 = 1000;

		// get preprepare hash
		if let Some((hash, height)) = self.env.preprepare(self.view) {
			log::trace!("{:?} preprepare_hash: {:?}", self.id, hash);

			self.current_height = height;
			self.message_log.lock().preprepare_hash = Some(hash);
		} else {
			// Even if we cannot get pre_prepre, it doesn't mean primary fails.
			//
			// The only way to determine primary fail is to
			// check last view round message to see if primary send
			// anything.
			self.multicast(Message::EmptyPrePrepare).await?;
		}

		Delay::new(Duration::from_millis(DELAY_PRE_PREPARE_MILLI)).await;

		Ok(())
	}

	/// Enter prepare phase
	///
	/// Called at end of pre-prepare or beginning of prepare.
	async fn prepare(&mut self) -> Result<(), E::Error> {
		log::trace!("{:?} start prepare", self.id);
		const DELAY_PREPARE_MILLI: u64 = 1000;

		// TODO: currently, change state without condition.
		if self.current_state == CurrentState::PrePrepare {
			self.change_state(CurrentState::Prepare);
		}

		let prepare_msg = Message::Prepare {
			view: self.view,
			seq_number: self.current_height,
			digest: self.message_log.lock().preprepare_hash.clone().unwrap().clone(),
		};

		self.multicast(prepare_msg).await?;

		Delay::new(Duration::from_millis(DELAY_PREPARE_MILLI)).await;
		log::trace!("{:?} end prepare", self.id);
		Ok(())
	}

	/// Enter Commit phase
	async fn commit(&mut self) -> Result<(), E::Error> {
		log::trace!("{:?} start commit", self.id);
		const DELAY_COMMIT_MILLI: u64 = 1000;

		if self.current_state == CurrentState::Prepare {
			self.validate_prepare();
		}

		let commit_msg = Message::Commit { view: self.view, seq_number: self.current_height };

		self.multicast(commit_msg).await?;

		Delay::new(Duration::from_millis(DELAY_COMMIT_MILLI)).await;
		log::trace!("{:?} end commit", self.id);
		Ok(())
	}

	fn validate_prepare(&mut self) -> bool {
		let c = self.message_log.lock().count_prepare();
		if helper::supermajority(self.voter_set.len(), c) {
			self.change_state(CurrentState::Commit)
		}
		true
	}

	fn validate_commit(&mut self) -> bool {
		let c = self.message_log.lock().count_commit();
		if helper::supermajority(self.voter_set.len(), c) {
			self.change_state(CurrentState::PrePrepare);
			return true;
		}
		false
	}

	async fn multicast(&mut self, msg: Message<E::Number, E::Hash>) -> Result<(), E::Error> {
		log::trace!("{:?} multicast message: {:?}", self.id, msg);
		dbg!(self.outgoing.send(msg).await)
	}

	fn primary_alive(&self) -> Result<(), E::Error> {
		let primary = self.voter_set.get_primary(self.view);
		log::trace!("{:?}, primary_alive: view: {}, primary: {:?}", self.id, self.view, primary);
		// TODO: .then_some(()).ok_or(Error::PrimaryFailure.into())
		self.message_log
			.lock()
			.contains_key(&primary)
			.then(|| ())
			.ok_or(Error::PrimaryFailure.into())
	}

	async fn progress(&mut self) -> Result<(), E::Error> {
		log::trace!("{:?} ViewRound::progress, view: {}", self.id, self.view);

		// preprepare
		self.preprepare().await?;

		if self.message_log.lock().preprepare_hash.is_none() {
			return self.primary_alive();
		}

		let hash = self
			.message_log
			.lock()
			.preprepare_hash
			.clone()
			.expect("We've already check this value in advance; qed");

		log::trace!("{:?} ViewRound:: get_preprepare: {:?}", self.id, hash);

		// prepare
		self.prepare().await?;

		// commit
		self.commit().await?;

		if self.validate_commit() {
			log::trace!("{:?} in view {} valid commit", self.id, self.view);
			self.env.finalize_block(hash, self.current_height);
		}

		log::trace!("{:?} ViewRound::END {:?}", self.id, self.view);

		self.primary_alive()
	}
}

#[cfg(test)]
mod tests {
	use std::{
		collections::BTreeMap,
		pin::Pin,
		task::{Context, Poll},
		time::Duration,
	};

	use super::*;
	use futures::{
		channel::mpsc::{self, UnboundedReceiver, UnboundedSender},
		executor::LocalPool,
		task::SpawnExt,
		Future, FutureExt, Sink, SinkExt, Stream, StreamExt,
	};
	use futures_timer::Delay;
	use parking_lot::Mutex;

	type ID = u64;
	type Signature = u64;
	type Hash = &'static str;
	type BlockNumber = u64;
	pub const GENESIS_HASH: &str = "genesis";
	const NULL_HASH: &str = "NULL";

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

	// type LogNetwork = CollectorNetwork<report::Log<&'static str, ID>>;
	type RoundNetwork = BroadcastNetwork<SignedMessage<u64, &'static str, Signature, ID>>;
	type GlobalMessageNetwork = BroadcastNetwork<GlobalMessage<ID>>;

	fn make_network() -> (Network, NetworkRouting) {
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
			node_id: ID,
		) -> (
			impl Stream<Item = Result<SignedMessage<u64, &'static str, Signature, ID>, Error>>,
			impl Sink<Message<u64, &'static str>, Error = Error>,
		) {
			log::trace!("make_round_comms, view_number: {}, node_id: {}", view_number, node_id);
			// let log_sender = self
			// 	.log_collector
			// 	.lock()
			// 	.add_node(move |log| report::Log { id: node_id.clone(), state: log });
			let mut rounds = self.rounds.lock();
			let round_comm = rounds.entry(view_number).or_insert_with(RoundNetwork::new).add_node(
				move |message| SignedMessage { message, signature: node_id, from: node_id },
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
			impl Stream<Item = Result<GlobalMessage<ID>, Error>>,
			impl Sink<GlobalMessage<ID>, Error = Error>,
		) {
			log::trace!("make_global_comms");
			let mut global = self.global.lock();
			let global_comm = global.add_node(|v| v);

			global_comm
		}
	}

	#[derive(Debug)]
	struct BlockRecord {
		/// Block height
		number: BlockNumber,
		/// Parent hash
		parent: Hash,
	}

	#[derive(Debug)]
	struct DummyChain {
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
			log::trace!("finalize_block: {:?}", block);
			if let Some(b) = self.inner.get(&block) {
				log::trace!("finalize block: {:?}", b);
				if self.inner.get(&b.parent).map(|p| p.number + 1 == b.number).unwrap_or(false) {
					self.finalized = (block, b.number);
					log::trace!("new finalized = {:?}", self.finalized);
					return true;
				} else {
					return false;
				}
			}

			false
		}
	}

	struct DummyEnvironment {
		local_id: ID,
		network: Network,
		listeners: Mutex<Vec<UnboundedSender<(Hash, BlockNumber)>>>,
		chain: Mutex<DummyChain>,
	}

	impl DummyEnvironment {
		pub fn new(network: Network, local_id: ID) -> Self {
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

	impl env::Environment for DummyEnvironment {
		type Timer = Box<dyn Future<Output = Result<(), Error>> + Unpin + Send>;
		type ID = ID;
		type Signature = Signature;
		type In = Box<
			dyn Stream<Item = Result<SignedMessage<u64, &'static str, Signature, ID>, Error>>
				+ Unpin
				+ Send,
		>;
		type Out = Pin<Box<dyn Sink<Message<u64, &'static str>, Error = Error> + Send>>;
		type Error = Error;
		type Hash = Hash;
		type Number = BlockNumber;

		fn voter_data(&self) -> communicate::VoterData<Self::ID> {
			communicate::VoterData { local_id: self.local_id }
		}

		fn round_data(
			&self,
			view: u64,
		) -> super::communicate::RoundData<Self::ID, Self::Timer, Self::In, Self::Out> {
			log::trace!("{:?} round_data view: {}", self.local_id, view);
			const GOSSIP_DURATION: Duration = Duration::from_millis(500);

			let (incomming, outgoing) = self.network.make_round_comms(view, self.local_id);
			communicate::RoundData {
				voter_id: self.local_id,
				prevote_timer: Box::new(Delay::new(GOSSIP_DURATION).map(Ok)),
				incoming: Box::new(incomming),
				outgoing: Box::pin(outgoing),
			}
		}

		fn finalize_block(
			&self,
			hash: Self::Hash,
			number: Self::Number,
			// commit: Message::Commit,
		) -> bool {
			log::trace!("{:?} finalize_block", self.local_id);
			self.chain.lock().finalize_block(hash);
			for i in self.listeners.lock().iter() {
				i.unbounded_send((hash, number)).unwrap();
			}

			true
		}

		fn preprepare(&self, _view: u64) -> Option<(Self::Hash, Self::Number)> {
			self.with_chain(|chain| {
				log::info!(
					"chain: {:?}, last_finalized: {:?}, next_to_be_finalized: {:?}",
					chain,
					chain.last_finalized(),
					chain.next_to_be_finalized()
				);
				chain.next_to_be_finalized().ok()
			})
		}
	}

	// - decode/encode test

	// state change test
	#[test]
	fn state_should_change_for_enough_prepare_msg() {
		// let (network, _) = make_network();
		// let voter_set = vec![0, 1, 2, 3];
		// let v1 = ViewRound::new(0, 1, voter_set, Arc::new(DummyEnvironment::new(network, 0)));
		//
		// assert_eq!(v1.current_state, CurrentState::PrePrepare);

		// v1.prepare();
		//
		// assert_eq!(v1.current_state, CurrentState::Prepare);

		// v1.handle_message(SignedMessage {
		// 	from: 1,
		// 	message: Message::Prepare { view: 0, digest: "block 1", seq_number: 1 },
		// 	signature: 123,
		// });
		//
		// assert_eq!(v1.current_state, CurrentState::Prepare);
		//
		// v1.handle_message(SignedMessage {
		// 	from: 2,
		// 	message: Message::Prepare { view: 0, digest: "block 1", seq_number: 1 },
		// 	signature: 123,
		// });
		//
		// assert_eq!(v1.current_state, CurrentState::Commit);
	}

	// communication test
	#[test]
	fn talking_to_myself() {
		let _ = simple_logger::init_with_level(log::Level::Info);

		let local_id = 5;
		let voter_set = VoterSet::new(vec![5]);

		let (network, routing_network) = make_network();
		let global_comms = network.make_global_comms();

		let env = Arc::new(DummyEnvironment::new(network, local_id));

		// init chain
		let last_finalized = env.with_chain(|chain| {
			chain.push_blocks(GENESIS_HASH, &["A", "B", "C", "D", "E"]);
			log::info!(
				"chain: {:?}, last_finalized: {:?}, next_to_be_finalized: {:?}",
				chain,
				chain.last_finalized(),
				chain.next_to_be_finalized()
			);
			chain.last_finalized()
		});

		let mut voter = Voter::new(env.clone(), voter_set, global_comms, 1, last_finalized);

		// run voter in background. scheduling it to shut down at the end.
		let finalized = env.finalized_stream();

		let mut pool = LocalPool::new();
		// futures::executor::block_on(testa(voter));
		pool.spawner()
			.spawn(async move {
				voter.run().await;
			})
			.unwrap();
		pool.spawner().spawn(routing_network).unwrap();

		// wait for the best block to finalized.
		pool.run_until(
			finalized
				.take_while(|&(_, n)| {
					log::info!("n: {}", n);
					futures::future::ready(n < 6)
				})
				.for_each(|v| {
					log::info!("v: {:?}", v);
					futures::future::ready(())
				}),
		)
	}

	#[test]
	fn multivoter() {
		let voters_num = 4;
		let online_voters_num = 3;
		let voter_set = VoterSet::new((0..voters_num).into_iter().collect());

		let (network, routing_network) = make_network();
		let mut pool = LocalPool::new();

		let finalized_streams = (0..online_voters_num)
			.map(|local_id| {
				// init chain
				let env = Arc::new(DummyEnvironment::new(network.clone(), local_id));
				let last_finalized = env.with_chain(|chain| {
					chain.push_blocks(GENESIS_HASH, &["A", "B", "C", "D", "E"]);
					chain.last_finalized()
				});

				let finalized = env.finalized_stream();
				let mut voter = Voter::new(
					env.clone(),
					voter_set.clone(),
					network.make_global_comms(),
					1,
					last_finalized,
				);

				pool.spawner()
					.spawn(async move {
						voter.run().await;
					})
					.unwrap();

				finalized
					.take_while(|&(_, n)| {
						log::trace!("n: {}", n);
						futures::future::ready(n < 6)
					})
					.for_each(|v| {
						log::trace!("v: {:?}", v);
						futures::future::ready(())
					})
			})
			.collect::<Vec<_>>();

		// run voter in background. scheduling it to shut down at the end.
		// futures::executor::block_on(testa(voter));
		pool.spawner().spawn(routing_network).unwrap();

		// wait for the best block to finalized.
		pool.run_until(futures::future::join_all(finalized_streams.into_iter()));
	}

	#[test]
	fn change_view_when_primary_fail() {
		simple_logger::init_with_level(log::Level::Trace).unwrap();
		let voters_num = 4;
		let online_voters_num = 3;
		let voter_set = VoterSet::new((0..voters_num).into_iter().collect());

		let (network, routing_network) = make_network();
		let mut pool = LocalPool::new();

		let finalized_streams = (1..online_voters_num + 1)
			.map(|local_id| {
				// init chain
				let env = Arc::new(DummyEnvironment::new(network.clone(), local_id));
				let last_finalized = env.with_chain(|chain| {
					chain.push_blocks(GENESIS_HASH, &["A", "B", "C", "D", "E"]);
					chain.last_finalized()
				});

				let finalized = env.finalized_stream();
				let mut voter = Voter::new(
					env.clone(),
					voter_set.clone(),
					network.make_global_comms(),
					// start from view 0
					0,
					last_finalized,
				);

				pool.spawner()
					.spawn(async move {
						voter.run().await;
					})
					.unwrap();

				finalized
					.take_while(|&(h, n)| {
						log::info!("finalized: hash: {}, block_height: {}", h, n);
						futures::future::ready(n < 6)
					})
					.for_each(|v| {
						log::info!("finalized: v: {:?}", v);
						futures::future::ready(())
					})
			})
			.collect::<Vec<_>>();

		// run voter in background. scheduling it to shut down at the end.
		// futures::executor::block_on(testa(voter));
		pool.spawner().spawn(routing_network).unwrap();

		// wait for the best block to finalized.
		pool.run_until(futures::future::join_all(finalized_streams.into_iter()));
	}

	#[test]
	fn view_did_not_change_when_primary_alive_without_preprepare() {}

	// #[test]
	// fn exposing_voter_state() {
	// 	let voters_num = 4;
	// 	let online_voters_num = 4;
	// 	let voter_set = VoterSet::new((0..voters_num).into_iter().collect());
	//
	// 	let (network, routing_network) = make_network();
	// 	let mut pool = LocalPool::new();
	//
	// 	let finalized_streams = (0..online_voters_num)
	// 		.map(|local_id| {
	// 			// init chain
	// 			let env = Arc::new(DummyEnvironment::new(network.clone(), local_id));
	// 			let last_finalized = env.with_chain(|chain| {
	// 				chain.push_blocks(GENESIS_HASH, &["A", "B", "C", "D", "E"]);
	// 				chain.last_finalized()
	// 			});
	//
	// 			let finalized = env.finalized_stream();
	// 			let mut voter = Voter::new(
	// 				env.clone(),
	// 				voter_set.clone(),
	// 				network.make_global_comms(),
	// 				1,
	// 				last_finalized,
	// 			);
	//
	// 			pool.spawner()
	// 				.spawn(async move {
	// 					voter.run().await;
	// 				})
	// 				.unwrap();
	//
	// 			finalized
	// 				.take_while(|&(_, n)| {
	// 					log::trace!("n: {}", n);
	// 					futures::future::ready(n < 6)
	// 				})
	// 				.for_each(|v| {
	// 					log::trace!("v: {:?}", v);
	// 					futures::future::ready(())
	// 				})
	// 		})
	// 		.collect::<Vec<_>>();
	//
	// 	// run voter in background. scheduling it to shut down at the end.
	// 	// futures::executor::block_on(testa(voter));
	// 	pool.spawner().spawn(routing_network).unwrap();
	//
	// 	// wait for the best block to finalized.
	// 	pool.run_until(futures::future::join_all(finalized_streams.into_iter()));
	// }
}
