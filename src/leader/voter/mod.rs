use std::{sync::Arc, time::Duration};

/// Callback used to pass information about the outcome of importing a given
/// message (e.g. vote, commit, catch up). Useful to propagate data to the
/// network after making sure the import is successful.
pub enum Callback<O> {
	/// Default value.
	Blank,
	/// Callback to execute given a processing outcome.
	Work(Box<dyn FnMut(O) + Send>),
}

impl<O> std::fmt::Debug for Callback<O> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Blank => write!(f, "Blank"),
			Self::Work(_arg0) => write!(f, "Work"),
		}
	}
}

#[cfg(any(test, feature = "test-helpers"))]
impl<O> Clone for Callback<O> {
	fn clone(&self) -> Self {
		Callback::Blank
	}
}

impl<O> Callback<O> {
	/// Do the work associated with the callback, if any.
	pub fn run(&mut self, o: O) {
		match self {
			Callback::Blank => {},
			Callback::Work(cb) => cb(o),
		}
	}
}

/// The outcome of processing a commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitProcessingOutcome {
	/// It was beneficial to process this commit.
	Good(GoodCommit),
	/// It wasn't beneficial to process this commit. We wasted resources.
	Bad(BadCommit),
}

#[cfg(any(test, feature = "test-helpers"))]
impl CommitProcessingOutcome {
	/// Returns a `Good` instance of commit processing outcome's opaque type. Useful for testing.
	pub fn good() -> CommitProcessingOutcome {
		CommitProcessingOutcome::Good(GoodCommit::new())
	}

	/// Returns a `Bad` instance of commit processing outcome's opaque type. Useful for testing.
	pub fn bad() -> CommitProcessingOutcome {
		CommitProcessingOutcome::Bad(CommitValidationResult::<(), ()>::default().into())
	}
}

/// The result of processing for a good commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoodCommit {}

impl GoodCommit {
	pub(crate) fn new() -> Self {
		GoodCommit {}
	}
}

/// The result of processing for a bad commit
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadCommit {
	num_commits: usize,
	num_duplicated_commits: usize,
	num_invalid_voters: usize,
}

impl BadCommit {
	/// Get the number of precommits
	pub fn num_commits(&self) -> usize {
		self.num_commits
	}

	/// Get the number of duplicated precommits
	pub fn num_duplicated(&self) -> usize {
		self.num_duplicated_commits
	}

	/// Get the number of invalid voters in the precommits
	pub fn num_invalid_voters(&self) -> usize {
		self.num_invalid_voters
	}
}

impl<H, N> From<CommitValidationResult<H, N>> for BadCommit {
	fn from(r: CommitValidationResult<H, N>) -> Self {
		BadCommit {
			num_commits: r.num_commits,
			num_duplicated_commits: r.num_duplicated_commits,
			num_invalid_voters: r.num_invalid_voters,
		}
	}
}

/// The outcome of processing a catch up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatchUpProcessingOutcome {
	/// It was beneficial to process this catch up.
	Good(GoodCatchUp),
	/// It wasn't beneficial to process this catch up, it is invalid and we
	/// wasted resources.
	Bad(BadCatchUp),
	/// The catch up wasn't processed because it is useless, e.g. it is for a
	/// round lower than we're currently in.
	Useless,
}

#[cfg(any(test, feature = "test-helpers"))]
impl CatchUpProcessingOutcome {
	/// Returns a `Bad` instance of catch up processing outcome's opaque type. Useful for testing.
	pub fn bad() -> CatchUpProcessingOutcome {
		CatchUpProcessingOutcome::Bad(BadCatchUp::new())
	}

	/// Returns a `Good` instance of catch up processing outcome's opaque type. Useful for testing.
	pub fn good() -> CatchUpProcessingOutcome {
		CatchUpProcessingOutcome::Good(GoodCatchUp::new())
	}
}

/// The result of processing for a good catch up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoodCatchUp {
	_priv: (), // lets us add stuff without breaking API.
}

impl GoodCatchUp {
	pub(crate) fn new() -> Self {
		GoodCatchUp { _priv: () }
	}
}

/// The result of processing for a bad catch up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadCatchUp {
	_priv: (), // lets us add stuff without breaking API.
}

impl BadCatchUp {
	pub(crate) fn new() -> Self {
		BadCatchUp { _priv: () }
	}
}

/// Communication between nodes that is not round-localized.
#[cfg_attr(any(test, feature = "test-helpers"), derive(Clone))]
#[derive(Debug)]
pub enum GlobalMessageIn<D, N, S, Id> {
	/// A commit message.
	Commit(u64, CompactCommit<D, N, S, Id>, Callback<CommitProcessingOutcome>),
	/// A catch up message.
	CatchUp(CatchUp<N, D, S, Id>, Callback<CatchUpProcessingOutcome>),
	/// multicast <view + 1, latest stable checkpoint, C: a set of pairs with the sequence number
	/// and digest of each checkpoint, P, Q, i>
	ViewChange(ViewChange<Id>),

	Empty,
}

impl<D, N, S, Id> Unpin for GlobalMessageIn<D, N, S, Id> {}

/// Communication between nodes that is not round-localized.
#[cfg_attr(any(test, feature = "test-helpers"), derive(Clone))]
#[derive(Debug)]
pub enum GlobalMessageOut<D, N, S, Id> {
	/// A commit message.
	Commit(
		u64,
		FinalizedCommit<N, D, S, Id>,
		// Callback<CommitProcessingOutcome>
	),
	/// multicast <view + 1, latest stable checkpoint, C: a set of pairs with the sequence number
	/// and digest of each checkpoint, P, Q, i>
	ViewChange(ViewChange<Id>),

	Empty,
}

pub(crate) mod helper {
	// TODO: eliminate this
	pub fn supermajority(n: usize, valid_number: usize) -> bool {
		n > 3 * (n - valid_number)
	}
}

type FinalizedNotification<E> = (
	<E as Environment>::Hash,
	<E as Environment>::Number,
	u64,
	FinalizedCommit<
		<E as Environment>::Number,
		<E as Environment>::Hash,
		<E as Environment>::Signature,
		<E as Environment>::Id,
	>,
);

pub struct Voter<E: Environment, GlobalIn, GlobalOut> {
	local_id: E::Id,
	env: Arc<E>,
	voters: VoterSet<E::Id>,
	inner: Arc<Mutex<InnerVoterState<E>>>,
	// finalized_notifications: UnboundedReceiver<FinalizedNotification<E>>,
	global_incoming: GlobalIn,
	global_outgoing: GlobalOut,
	// TODO: consider wrap this around sth.
	last_finalized_target: (E::Number, E::Hash),
	current_view_number: u64,
}

// QUESTION: why 'a
impl<'a, E: 'a, GlobalIn, GlobalOut> Voter<E, GlobalIn, GlobalOut>
where
	E: Environment + Sync + Send,
	GlobalIn: Stream<Item = Result<GlobalMessageIn<E::Hash, E::Number, E::Signature, E::Id>, E::Error>>
		+ Unpin,
	GlobalOut:
		Sink<GlobalMessageOut<E::Hash, E::Number, E::Signature, E::Id>, Error = E::Error> + Unpin,
{
	/// Returns an object allowing to query the voter state.
	pub fn voter_state(&self) -> Box<dyn VoterState<E::Hash, E::Id> + 'a + Send + Sync>
	where
		<E as Environment>::Signature: Send,
		<E as Environment>::Id: std::hash::Hash + Send,
		<E as Environment>::Timer: Send,
		<E as Environment>::Out: Send,
		<E as Environment>::In: Send,
		<E as Environment>::Number: Send,
		<E as Environment>::Hash: Send,
	{
		Box::new(SharedVoterState(self.inner.clone()))
	}
}

impl<E: Environment, GlobalIn, GlobalOut> Voter<E, GlobalIn, GlobalOut>
where
	GlobalIn: Stream<Item = Result<GlobalMessageIn<E::Hash, E::Number, E::Signature, E::Id>, E::Error>>
		+ Unpin,
	GlobalOut:
		Sink<GlobalMessageOut<E::Hash, E::Number, E::Signature, E::Id>, Error = E::Error> + Unpin,
{
	pub fn new(
		env: Arc<E>,
		voters: VoterSet<E::Id>,
		global_comms: (GlobalIn, GlobalOut),
		current_view_number: u64,
		last_round_base: (E::Number, E::Hash),
	) -> Self {
		// let (finalized_sender, finalized_notifications) = mpsc::unbounded();
		let voter_data = env.voter_data();

		let best_view = ViewRound::new(
			current_view_number,
			last_round_base.clone(),
			voters.clone(),
			env.clone(),
		);

		Self {
			local_id: voter_data.local_id,
			env: env.clone(),
			voters,
			global_incoming: global_comms.0,
			global_outgoing: global_comms.1,
			last_finalized_target: last_round_base,
			inner: Arc::new(Mutex::new(InnerVoterState { best_view })),
			current_view_number,
		}
	}

	pub fn new_view_round(&self) -> ViewRound<E> {
		let voter_set = self.voters.clone();
		ViewRound::new(
			self.current_view_number,
			self.last_finalized_target.clone(),
			voter_set,
			self.env.clone(),
		)
	}

	pub async fn process_voting_round(
		view_round: &mut ViewRound<E>,
	) -> Result<(E::Number, E::Hash), E::Error> {
		let mut inner_incoming = view_round.incoming.take().expect("inner_incoming must exist.");
		let message_log = view_round.message_log.clone();

		// FIXME: optimize
		// log::trace!("{:?} process_voting_round {:?}", inner.id, inner.view);
		let a = ViewRound::<E>::process_incoming(&mut inner_incoming, message_log).fuse();
		let b = view_round.progress().fuse();

		futures::pin_mut!(a, b);

		futures::select! {
			a = a => {
				log::trace!(target: "afp", "a ready {:?}", a);
				Err(super::Error::IncomingClosed.into())
			}
			b = b => {
				log::trace!(target: "afp", "b ready {:?}", b);
				b
			}
		}
	}

	pub async fn change_view(
		current_view: u64,
		id: E::Id,
		global_outgoing: &mut GlobalOut,
		global_incoming: &mut GlobalIn,
		voters: &VoterSet<E::Id>,
	) -> Result<u64, E::Error> {
		const DELAY_VIEW_CHANGE_WAIT: u64 = 1000;
		let mut new_view = current_view + 1;
		loop {
			log::info!(target: "afp", "id: {:?}, start view_change: {new_view}", id);

			// create new view change message
			let view_change = ViewChange::new(new_view, id.clone());
			let mut log = HashMap::<E::Id, ViewChange<E::Id>>::new();

			global_outgoing.send(GlobalMessageOut::ViewChange(view_change.clone())).await?;

			let mut timeout = Delay::new(Duration::from_millis(DELAY_VIEW_CHANGE_WAIT)).fuse();

			loop {
				// Stream.next with timeout
				futures::select! {
					_ = timeout => {
						break;
					},
					msg = global_incoming.try_next().fuse() => {
						let msg = msg?.unwrap();
						if let GlobalMessageIn::ViewChange (msg) = msg {
							if new_view == msg.new_view {
								log::trace!(target: "afp", "{:?} save valid viewChange {:?}", id, msg);
								log.insert(msg.id.clone(), msg);
							}
						}
					}
				}
			}

			if log.len() >= voters.threshould() {
				log::info!(target: "afp", "id: {:?}, successfully view_change: {new_view}", id);
				return Ok(new_view)
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
		log::trace!(target: "afp", "{:?} Voter::run", self.local_id);

		loop {
			let mut best_view = self.inner.lock().best_view.take_clone();

			// NOTE: this is a hack to make sure new network is activated, by sending a empty
			// mesage to the network.
			//
			// FIXME: This message could be eliminated by implmenting following step:
			// a) always create next-view network aggressively, even in the case of a view change.
			// a.1) In detail, the network should be created when the view number changed.
			let _ = self.global_outgoing.send(GlobalMessageOut::Empty).await;

			// run both global_incoming and view_round
			// wait one of them return
			let result =
				Voter::<E, GlobalIn, GlobalOut>::process_voting_round(&mut best_view).await;

			// restart new voting round.
			log::trace!(target: "afp", "{:?} current voting round finish: {:?}", self.local_id, result);
			if let Err(e) = result {
				log::warn!(target: "afp", "Error throw by ViewRound: {:?}", e);
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
			} else {
				self.last_finalized_target = result.unwrap();
			}

			// Initiate a new view round.
			self.inner.lock().best_view = self.new_view_round();
		}
	}

	// pub async fn process_current_consensus(&self) {}
}
/// Trait for querying the state of the voter. Used by `Voter` to return a queryable object
/// without exposing too many data types.
pub trait VoterState<Hash, Id: Eq + std::hash::Hash> {
	/// Returns a plain data type, `report::VoterState`, describing the current state
	/// of the voter relevant to the voting process.
	fn get(&self) -> report::VoterState<Hash, Id>;
}

/// Communication helper data
pub mod communicate {
	/// Data necessary to create a voter.
	pub struct VoterData<Id> {
		/// Local voter id.
		pub local_id: Id,
	}

	/// Data necessary to participate in a round.
	pub struct RoundData<Id, Input, Output> {
		/// Local voter id
		pub voter_id: Id,
		// Timer before prevotes can be cast. This should be Start + 2T
		// where T is the gossip time estimate.
		// pub prevote_timer: Timer,
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
	use super::super::CurrentState;
	use std::collections::{HashMap, HashSet};

	#[derive(Clone, Debug, PartialEq, Eq)]
	pub struct ViewState<Hash, Id: Eq + std::hash::Hash> {
		pub state: CurrentState,
		pub total_voters: usize,
		pub threshold: usize,
		pub preprepare_hash: Option<Hash>,
		/// The identities of nodes that have cast prepare so far.
		pub prepare_ids: HashSet<Id>,
		pub commit_ids: HashSet<Id>,
	}

	#[derive(Clone, Debug)]
	pub struct VoterState<Hash, Id: Eq + std::hash::Hash> {
		/// Voting rounds running in the background.
		pub background_views: HashMap<u64, ViewState<Hash, Id>>,
		/// The current best voting view.
		pub best_view: (u64, ViewState<Hash, Id>),
	}

	#[derive(Clone, Debug)]
	pub struct Log<Hash, Id: Eq + std::hash::Hash> {
		pub id: Id,
		pub state: ViewState<Hash, Id>,
	}
}

// The inner state of a voter aggregating the currently running round state
// (i.e. best and background rounds). This state exists separately since it's
// useful to wrap in a `Arc<Mutex<_>>` for sharing.
pub struct InnerVoterState<E>
where
	// H: Clone + Ord + std::fmt::Debug,
	// N: BlockNumberOps,
	E: Environment,
{
	best_view: ViewRound<E>,
	// past_views: PastRounds<H, N, E>,
}

struct SharedVoterState<E>(Arc<Mutex<InnerVoterState<E>>>)
where
	// H: Clone + Ord + std::fmt::Debug,
	// N: BlockNumberOps,
	E: Environment;

impl<E> VoterState<E::Hash, E::Id> for SharedVoterState<E>
where
	// H: Clone + Eq + Ord + std::fmt::Debug,
	// N: BlockNumberOps,
	E: Environment,
	// <E as Environment>::Id: Hash,
{
	fn get(&self) -> report::VoterState<E::Hash, E::Id> {
		let to_view_state = |view_round: &ViewRound<E>| {
			let preprepare = view_round.message_log.lock().target.clone();
			let prepare_ids = view_round.message_log.lock().prepare.keys().cloned().collect();
			let commit_ids = view_round.message_log.lock().commit.keys().cloned().collect();
			let state = view_round.message_log.lock().current_state;
			(
				view_round.view,
				report::ViewState {
					state,
					total_voters: view_round.voter_set.len().get(),
					threshold: view_round.voter_set.threshould,
					preprepare_hash: preprepare.map(|(n, h)| h),
					prepare_ids,
					commit_ids,
				},
			)
		};

		let inner = self.0.lock();
		let best_view = to_view_state(&inner.best_view);
		// let background_rounds = inner.past_rounds.voting_rounds().map(to_round_state).collect();

		report::VoterState { best_view, background_views: HashMap::new() }
	}
}

/// Environment for consensus.
///
/// Including network, storage and other info (that cannot get directly via consensus algo).
use std::collections::HashMap;

use futures::{Future, FutureExt, Sink, SinkExt, Stream, TryStreamExt};
use futures_timer::Delay;
use parking_lot::Mutex;

use crate::{
	leader::{Commit, PrePrepare, Prepare, ViewChange},
	BlockNumberOps,
};

use self::communicate::RoundData;

use super::{
	CatchUp, CommitValidationResult, CompactCommit, CurrentState, Error, FinalizedCommit, Message,
	SignedCommit, SignedMessage, Storage, VoterSet,
};

/// Necessary environment for a voter.
///
/// This encapsulates the database and networking layers of the chain.
pub trait Environment {
	/// Associated timer type for the environment. See also [`Self::round_data`] and
	/// [`Self::round_commit_timer`].
	type Timer: Future<Output = Result<(), Self::Error>> + Unpin;
	/// The associated Id for the Environment.
	type Id: Clone + Eq + std::hash::Hash + Ord + std::fmt::Debug;
	/// The associated Signature type for the Environment.
	type Signature: Eq + Clone + core::fmt::Debug;
	/// Associated future type for the environment used when asynchronously computing the
	/// best chain to vote on. See also [`Self::best_chain_containing`].
	type BestChain: Future<Output = Result<Option<(Self::Number, Self::Hash)>, Self::Error>>
		+ Send
		+ Unpin;
	/// The input stream used to communicate with the outside world.
	type In: Stream<
			Item = Result<
				SignedMessage<Self::Number, Self::Hash, Self::Signature, Self::Id>,
				Self::Error,
			>,
		> + Unpin;
	/// The output stream used to communicate with the outside world.
	type Out: Sink<Message<Self::Number, Self::Hash>, Error = Self::Error> + Unpin;
	/// The associated Error type.
	type Error: From<Error> + ::std::error::Error;
	/// Hash type used in blockchain or digest.
	type Hash: Eq + Clone + core::fmt::Debug;
	/// The block number type.
	type Number: BlockNumberOps;

	/// Get Voter data.
	fn voter_data(&self) -> communicate::VoterData<Self::Id>;

	/// Get round data.
	fn round_data(&self, view: u64) -> communicate::RoundData<Self::Id, Self::In, Self::Out>;

	/// preprepare
	fn preprepare(&self, view: u64, block: Self::Hash) -> Self::BestChain;

	/// Finalize a block.
	// TODO: maybe async?
	fn finalize_block(
		&self,
		view: u64,
		hash: Self::Hash,
		number: Self::Number,
		f_commit: FinalizedCommit<Self::Number, Self::Hash, Self::Signature, Self::Id>,
	) -> Result<(), Self::Error>;
}

pub struct Logger<E>
where
	E: Environment,
{
	storage: Mutex<HashMap<E::Id, Vec<report::ViewState<E::Hash, E::Id>>>>,
}

impl<E> Logger<E>
where
	E: Environment,
{
	pub fn push_view_state(&mut self, id: E::Id, state: report::ViewState<E::Hash, E::Id>) {
		self.storage.lock().entry(id).or_insert(Vec::new()).push(state);
	}
}

/// Logic for a voter on a specific round.
/// Similar to [`voting_round::VotingRound`]
pub struct ViewRound<E: Environment> {
	env: Arc<E>,
	voter_set: VoterSet<E::Id>,
	message_log: Arc<Mutex<Storage<E::Number, E::Hash, E::Signature, E::Id>>>,
	view: u64,
	id: E::Id,
	incoming: Option<E::In>,
	outgoing: Option<E::Out>,
	// finalized_sender: UnboundedSender<FinalizedNotification<E>>,
}

impl<E> ViewRound<E>
where
	E: Environment,
{
	// TODO: maybe delete some fields?
	pub fn new(
		view: u64,
		last_round_base: (E::Number, E::Hash),
		voter_set: VoterSet<E::Id>,
		env: Arc<E>,
	) -> Self {
		let RoundData { voter_id, incoming, outgoing, .. } = env.round_data(view);

		ViewRound {
			env,
			view,
			voter_set,
			message_log: Arc::new(Mutex::new(Storage::new(last_round_base))),
			id: voter_id,
			incoming: Some(incoming),
			// TODO: why bufferd
			outgoing: Some(outgoing),
		}
	}

	fn take_clone(&mut self) -> Self {
		ViewRound {
			env: self.env.clone(),
			voter_set: self.voter_set.clone(),
			message_log: self.message_log.clone(),
			view: self.view,
			id: self.id.clone(),
			incoming: self.incoming.take(),
			outgoing: self.outgoing.take(),
		}
	}

	fn change_state(&self, state: CurrentState) {
		// TODO: push log
		self.message_log.lock().current_state = state;
	}

	async fn process_incoming(
		incoming: &mut E::In,
		log: Arc<Mutex<Storage<E::Number, E::Hash, E::Signature, E::Id>>>,
	) -> Result<(), E::Error> {
		// FIXME: optimize
		log::trace!(target: "afp", "start of process_incoming");
		while let Some(msg) = incoming.try_next().await? {
			log::trace!(target: "afp", "process_incoming: {:?}", msg);
			let SignedMessage { message, signature, id } = msg;
			log.lock().save_message(id, message, signature);
		}

		log::trace!(target: "afp", "end of process_incoming");
		Ok(())
	}

	fn is_primary(&self) -> bool {
		self.voter_set.get_primary(self.view) == self.id
	}

	fn validate_preprepare(&mut self) -> bool {
		log::trace!(target: "afp", "{:?} start validate_preprepare", self.id);
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

	fn update_current_target(&self, target: (E::Number, E::Hash)) {
		self.message_log.lock().target = Some(target);
	}

	async fn preprepare(&mut self) -> Result<(), E::Error> {
		const DELAY_PRE_PREPARE_MILLI: u64 = 1000;

		let last_round_base = self.message_log.lock().last_round_base.clone();

		// get preprepare hash
		let (height, hash) = self.env.preprepare(self.view, last_round_base.1).await?.unwrap();
		log::trace!(target: "afp", "{:?} preprepare_hash: {:?}", self.id, hash);

		self.update_current_target((height, hash.clone()));

		let preprepare = PrePrepare::new(self.view, height, hash);
		self.multicast(Message::PrePrepare(preprepare)).await?;

		Delay::new(Duration::from_millis(DELAY_PRE_PREPARE_MILLI)).await;

		Ok(())
	}

	/// Enter prepare phase
	///
	/// Called at end of pre-prepare or beginning of prepare.
	async fn prepare(&mut self) -> Result<(), E::Error> {
		log::trace!(target: "afp", "{:?} start prepare", self.id);
		const DELAY_PREPARE_MILLI: u64 = 1000;

		// TODO: currently, change state without condition.
		self.change_state(CurrentState::Prepare);

		let (height, hash) = self.message_log.lock().target.clone().unwrap();

		let prepare_msg = Message::Prepare(Prepare::new(self.view, height, hash));

		self.multicast(prepare_msg).await?;

		Delay::new(Duration::from_millis(DELAY_PREPARE_MILLI)).await;
		log::trace!(target: "afp", "{:?} end prepare", self.id);
		Ok(())
	}

	/// Enter Commit phase
	async fn commit(&mut self) -> Result<(), E::Error> {
		log::trace!(target: "afp", "{:?} start commit", self.id);
		const DELAY_COMMIT_MILLI: u64 = 1000;

		self.validate_prepare();

		let (height, hash) = self.message_log.lock().target.clone().unwrap();

		let commit = Commit::new(self.view, height, hash);

		let commit_msg = Message::Commit(commit);

		self.multicast(commit_msg).await?;

		Delay::new(Duration::from_millis(DELAY_COMMIT_MILLI)).await;
		log::trace!(target: "afp", "{:?} end commit", self.id);
		Ok(())
	}

	fn validate_prepare(&mut self) -> bool {
		let c = self.message_log.lock().count_prepares();
		if helper::supermajority(self.voter_set.len().get(), c) {
			self.change_state(CurrentState::Commit)
		}
		true
	}

	fn validate_commit(
		&mut self,
	) -> Option<FinalizedCommit<E::Number, E::Hash, E::Signature, E::Id>> {
		let c = self.message_log.lock().count_commits();
		if helper::supermajority(self.voter_set.len().get(), c) {
			self.change_state(CurrentState::PrePrepare);

			let (target_height, target_hash) = self.message_log.lock().target.clone().unwrap();
			let commits = self
				.message_log
				.lock()
				.commit
				.iter()
				.map(|(id, (commit, sig))| SignedCommit {
					commit: commit.clone(),
					signature: sig.clone(),
					id: id.clone(),
				})
				.collect();

			let f_commit = FinalizedCommit { target_hash, target_number: target_height, commits };

			return Some(f_commit)
		}
		log::info!(target: "afp", "{:?} commit not enough", self.id);
		None
	}

	async fn multicast(&mut self, msg: Message<E::Number, E::Hash>) -> Result<(), E::Error> {
		log::trace!(target: "afp", "{:?} multicast message: {:?}", self.id, msg);
		self.outgoing.as_mut().unwrap().send(msg).await
	}

	fn primary_alive(&self) -> Result<(), E::Error> {
		let primary = self.voter_set.get_primary(self.view);
		log::trace!(target: "afp", "{:?}, primary_alive: view: {}, primary: {:?}", self.id, self.view, primary);
		// TODO: .then_some(()).ok_or(Error::PrimaryFailure.into())
		self.message_log
			.lock()
			.contains_key(&primary)
			.then(|| ())
			.ok_or(Error::PrimaryFailure.into())
	}

	async fn progress(&mut self) -> Result<(E::Number, E::Hash), E::Error> {
		log::trace!(target: "afp", "{:?} ViewRound::progress, view: {}", self.id, self.view);

		// preprepare
		self.preprepare().await?;

		if self.message_log.lock().target.is_none() {
			self.primary_alive()?;
			unreachable!();
		}

		let (height, hash) = self
			.message_log
			.lock()
			.target
			.clone()
			.expect("We've already check this value in advance; qed");

		log::trace!(target: "afp", "{:?} ViewRound:: get_preprepare: {:?}", self.id, hash);

		// prepare
		self.prepare().await?;

		// commit
		self.commit().await?;

		if let Some(f_commit) = self.validate_commit() {
			log::trace!(target: "afp", "{:?} in view {} valid commit", self.id, self.view);
			let _ = self.env.finalize_block(self.view, hash, height, f_commit);
		}

		log::trace!(target: "afp", "{:?} ViewRound::END {:?}", self.id, self.view);

		self.primary_alive()?;

		Ok(self
			.message_log
			.lock()
			.target
			.clone()
			.expect("We've already check this value in advance; qed"))
	}
}

#[cfg(test)]
mod tests {
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
		let _ = simple_logger::init_with_level(log::Level::Trace);

		let local_id = 5;
		let voter_set = VoterSet::new(vec![5]).unwrap();

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
		let voter_set = VoterSet::new((0..voters_num).into_iter().collect()).unwrap();

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
		// simple_logger::init_with_level(log::Level::Trace).unwrap();
		let voters_num = 4;
		let online_voters_num = 3;
		let voter_set = VoterSet::new((0..voters_num).into_iter().collect()).unwrap();

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

	use std::sync::Arc;

	use futures::{executor::LocalPool, task::SpawnExt, StreamExt};

	use crate::leader::{
		testing::{
			chain::GENESIS_HASH,
			environment::{make_network, DummyEnvironment},
		},
		VoterSet,
	};

	use super::Voter;
}
