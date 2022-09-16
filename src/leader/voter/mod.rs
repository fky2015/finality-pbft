use std::{sync::Arc, task::Poll, time::Duration};

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
	/// A workaround for test network.
	Empty,
}

impl<D, N, S, Id> PartialEq for GlobalMessageIn<D, N, S, Id> {
	fn eq(&self, other: &Self) -> bool {
		match (self, other) {
			(Self::Empty, Self::Empty) => true,
			_ => false,
		}
	}
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

/// Storage ViewChange message and validate.
#[derive(Debug)]
pub(crate) struct PeerViewChange<Id, N, H, S> {
	// <ID, BEST_VIEW_CHANGE>
	inner: HashMap<Id, u64>,
	best_finzalized: Option<FinalizedCommit<N, H, S, Id>>,
}

impl<Id: Eq + std::hash::Hash + Clone, N: Clone, H: Clone, S: Clone> PeerViewChange<Id, N, H, S> {
	pub fn new() -> Self {
		PeerViewChange { inner: HashMap::new(), best_finzalized: None }
	}

	pub fn insert(&mut self, id: Id, view: u64) {
		if let Some(old_view) = self.inner.get(&id).clone() {
			if *old_view > view {
				return
			} else {
				self.inner.insert(id, view);
			}
		} else {
			self.inner.insert(id, view);
		}
	}

	/// Count the number of peers in the tergeting view.
	pub fn count_view_change(&self, target_view: u64) -> usize {
		self.inner.iter().filter(|(_, &v)| v == target_view).count()
	}

	pub fn prune(&mut self) {
		self.inner.clear();
	}

	/// Return if a peer is in specific view.
	pub fn exist(&self, id: &Id, view: u64) -> bool {
		self.inner.get(id).map(|&v| v == view).unwrap_or(false)
	}

	/// Return if higher view exist.
	pub fn exist_higher(&self, id: &Id, view: u64) -> bool {
		self.inner.get(id).map(|&v| v <= view).unwrap_or(false)
	}

	/// Return hightest valid view number.
	pub fn exist_valid_view(
		&self,
		lowest_view: u64,
		threshold: usize,
	) -> Option<(u64, Option<FinalizedCommit<N, H, S, Id>>)> {
		let mut count = HashMap::new();
		self.inner.iter().filter(|(_, &v)| v >= lowest_view).for_each(|(_, &v)| {
			count.entry(v).and_modify(|c| *c += 1).or_insert(1);
		});

		count
			.iter()
			.filter(|(_, &c)| c >= threshold)
			.map(|(v, _)| *v)
			.next()
			.map(|v| (v, self.best_finzalized.clone()))
	}

	/// For catch-up message.
	pub fn insert_best_common_finalized(&mut self, best_finalized: FinalizedCommit<N, H, S, Id>) {
		self.best_finzalized = Some(best_finalized);
	}
}

/// A voter can cast message in both global and rounds (views).
/// Implment pBFT.
pub struct Voter<E: Environment, GlobalIn, GlobalOut> {
	/// Local Id.
	local_id: E::Id,
	/// Environment.
	env: Arc<E>,
	/// Current Voter Set.
	voters: VoterSet<E::Id>,
	/// Inner voter state (for view).
	inner: Arc<Mutex<InnerVoterState<E>>>,
	/// Global message in.
	global_incoming: Option<GlobalIn>,
	/// Global message out.
	global_outgoing: GlobalOut,
	/// Last block finalized.
	last_finalized_target: (E::Number, E::Hash),
	/// Current view.
	current_view_number: u64,
	/// Storing Peers view when starting a view change.
	peer_view: Arc<Mutex<PeerViewChange<E::Id, E::Number, E::Hash, E::Signature>>>,
}

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

		log::info!("Voter::new voter set {:?}", voters);

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
			global_incoming: Some(global_comms.0),
			global_outgoing: global_comms.1,
			last_finalized_target: last_round_base,
			inner: Arc::new(Mutex::new(InnerVoterState { best_view })),
			current_view_number,
			peer_view: Arc::new(Mutex::new(PeerViewChange::new())),
		}
	}

	/// Init a new view.
	fn new_view_round(&self) -> ViewRound<E> {
		let voter_set = self.voters.clone();
		ViewRound::new(
			self.current_view_number,
			self.last_finalized_target.clone(),
			voter_set,
			self.env.clone(),
		)
	}

	/// Process a round in current view. PBFT Implmentation.
	pub async fn process_voting_round(
		view_round: &mut ViewRound<E>,
	) -> Result<(E::Number, E::Hash), E::Error> {
		let mut inner_incoming = view_round.incoming.take().expect("inner_incoming must exist.");
		let message_log = view_round.message_log.clone();
		let view = view_round.view;

		let res = {
			let a_fused =
				ViewRound::<E>::process_incoming(&mut inner_incoming, message_log, view).fuse();
			let b_fused = view_round.progress().fuse();

			futures::pin_mut!(a_fused, b_fused);

			futures::select! {
				a = a_fused => {
					log::trace!(target: "afp", "a ready {:?}", a);
					Err(super::Error::IncomingClosed.into())
				}
				b = b_fused => {
					log::trace!(target: "afp", "b ready {:?}", b);
					b
				}
			}
		};

		// Move the ownership back.
		view_round.incoming = Some(inner_incoming);

		res
	}

	/// Change view. PBFT implementation.
	pub async fn change_view(&mut self) -> Result<u64, E::Error> {
		const DELAY_VIEW_CHANGE_WAIT: u64 = 500;

		self.inner.lock().best_view.message_log.lock().current_state = CurrentState::ViewChange;

		let new_view = self.current_view_number + 1;
		loop {
			log::info!(target: "afp", "id: {:?}, start view_change: {new_view}", self.local_id);

			// create new view change message
			let view_change = ViewChange::new(new_view, self.local_id.clone());

			self.global_outgoing
				.send(GlobalMessageOut::ViewChange(view_change.clone()))
				.await?;

			// wait for timeout.
			Delay::new(Duration::from_millis(DELAY_VIEW_CHANGE_WAIT)).await;

			// Our view change message is not included in the log.
			let result =
				self.peer_view.lock().exist_valid_view(new_view, self.voters.threshold() - 1);
			// TODO: also, try to process catch-up message.
			if let Some(new_view_info) = result {
				if let Some(f_commit) = new_view_info.1.clone() {
					log::info!(target: "afp", "id: {:?}, collect new catch up from peers: {:?}", self.local_id, f_commit);
					self.last_finalized_target =
						(f_commit.target_number.clone(), f_commit.target_hash.clone());

					let _ = self.env.finalize_block(
						new_view_info.0,
						self.last_finalized_target.1.clone(),
						self.last_finalized_target.0.clone(),
						f_commit,
					);
				};
				// TODO: maybe not prune those who higher than new_view.
				self.peer_view.lock().prune();
				log::info!(target: "afp", "id: {:?}, successfully view_change: {:?}", self.local_id, new_view_info);

				self.inner.lock().best_view.message_log.lock().current_state =
					CurrentState::PrePrepare;
				return Ok(new_view_info.0)
			}
		}
	}

	/// This is a entrance of the consensus.
	pub async fn run(&mut self) {
		log::trace!(target: "afp", "{:?} Voter::run", self.local_id);

		let global_incoming = Self::process_global_incoming(
			self.global_incoming.take().unwrap(),
			self.peer_view.clone(),
			self.local_id.clone(),
		)
		.fuse();
		let run_voter = self.run_voter().fuse();

		futures::pin_mut!(run_voter, global_incoming);
		// This should never return.
		futures::future::select(run_voter, global_incoming).await;

		log::warn!(target: "afp", "voter stopped.");
	}

	/// Run voter.
	pub(crate) async fn run_voter(&mut self) {
		let mut best_view = self.inner.lock().best_view.take_clone();

		loop {
			// NOTE: this is a hack to make sure new network is activated, by sending a empty
			// mesage to the network.
			//
			// FIXME: This message could be eliminated by implmenting following step:
			// a) always create next-view network aggressively, even in the case of a view change.
			// a.1) In detail, the network should be created when the view number changed.
			let _ = self.global_outgoing.send(GlobalMessageOut::Empty).await;

			let result =
				Voter::<E, GlobalIn, GlobalOut>::process_voting_round(&mut best_view).await;

			// restart new voting round.
			log::trace!(target: "afp", "{:?} current voting round finish: {:?}", self.local_id, result);
			if let Err(e) = result {
				log::warn!(target: "afp", "Error throw by ViewRound: {:?}", e);

				// Change to new view.
				let new_view = self.change_view().await.unwrap();

				self.current_view_number = new_view;

				// Initiate a new view round.
				self.inner.lock().best_view = self.new_view_round();
				best_view = self.inner.lock().best_view.take_clone();
			} else {
				self.last_finalized_target = result.unwrap();
			}
		}
	}

	/// Process global incoming message.
	pub(crate) async fn process_global_incoming(
		mut global_incoming: GlobalIn,
		peer_view_change: Arc<Mutex<PeerViewChange<E::Id, E::Number, E::Hash, E::Signature>>>,
		local_id: E::Id,
	) {
		log::trace!(target: "afp", "Voter::process_global_incoming");
		loop {
			match global_incoming.try_next().await.unwrap().unwrap() {
				GlobalMessageIn::Commit(_, _, _) => todo!(),
				GlobalMessageIn::CatchUp(CatchUp { view_number, commits, .. }, _cb) => {
					// catch up will only be accepted when voter is during a view changing (not
					// voting).
					// This could be checked by finding self view change message.
					// if peer_view_change.lock().exist(&local_id, view_number) {
					// 1. TODO: validate

					// 2. update current view change
					let target = commits
						.iter()
						.next()
						.map(|c| (c.commit.target_number, c.commit.target_hash.clone()))
						.unwrap();
					commits.iter().for_each(|SignedCommit { id, .. }| {
						peer_view_change.lock().insert(id.clone(), view_number);
					});

					log::trace!(target: "afp", "{:?} catch up message: {:?}", local_id, (view_number, target.clone()));

					let f_commit =
						FinalizedCommit { target_number: target.0, target_hash: target.1, commits };
					peer_view_change.lock().insert_best_common_finalized(f_commit);
				},
				GlobalMessageIn::ViewChange(ViewChange { new_view, id }) => {
					if id == local_id {
						continue
					}
					peer_view_change.lock().insert(id, new_view);
					log::trace!(target: "afp", "{:?} view change message: {:?}", local_id, new_view);
				},
				GlobalMessageIn::Empty => {},
			};

			log::trace!(target: "afp", "received a global_incoming msg");
		}
	}
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

/// The inner state of a voter aggregating the currently running round state
/// (i.e. best and background rounds). This state exists separately since it's
/// useful to wrap in a `Arc<Mutex<_>>` for sharing.
pub struct InnerVoterState<E>
where
	E: Environment,
{
	best_view: ViewRound<E>,
	// past_views: PastRounds<H, N, E>,
}

struct SharedVoterState<E>(Arc<Mutex<InnerVoterState<E>>>)
where
	E: Environment;

impl<E> VoterState<E::Hash, E::Id> for SharedVoterState<E>
where
	E: Environment,
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
					threshold: view_round.voter_set.threshold,
					preprepare_hash: preprepare.map(|(_n, h)| h),
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
	SignedCommit, SignedMessage, State, Storage, VoterSet,
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

	/// Similar to `completed`
	fn complete_f_commit(
		&self,
		view: u64,
		state: State<Self::Number, Self::Hash>,
		base: (Self::Number, Self::Hash),
		f_commit: FinalizedCommit<Self::Number, Self::Hash, Self::Signature, Self::Id>,
	) -> Result<(), Self::Error>;

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
	/// Storage state for the voter.
	message_log: Arc<Mutex<Storage<E::Number, E::Hash, E::Signature, E::Id>>>,
	/// PBFT view number.
	view: u64,
	/// The id of the voter.
	id: E::Id,
	/// Receiver for incoming messages.
	incoming: Option<E::In>,
	/// Sender for outgoing messages.
	outgoing: Option<E::Out>,
}

impl<E> ViewRound<E>
where
	E: Environment,
{
	pub fn new(
		view: u64,
		last_round_base: (E::Number, E::Hash),
		voter_set: VoterSet<E::Id>,
		env: Arc<E>,
	) -> Self {
		let RoundData { voter_id, incoming, outgoing } = env.round_data(view);

		ViewRound {
			env,
			view,
			voter_set: voter_set.clone(),
			message_log: Arc::new(Mutex::new(Storage::new(last_round_base, voter_set))),
			id: voter_id,
			incoming: Some(incoming),
			// TODO: why bufferd
			outgoing: Some(outgoing),
		}
	}

	/// Clone this `ViewRound`, but take the ownership of the `incoming` and `outgoing`.
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

	/// Update the PBFT state.
	fn change_state(&self, state: CurrentState) {
		// TODO: trace push log?
		self.message_log.lock().current_state = state;
	}

	/// Process incoming messages in a round in the view.
	async fn process_incoming(
		incoming: &mut E::In,
		log: Arc<Mutex<Storage<E::Number, E::Hash, E::Signature, E::Id>>>,
		view: u64,
	) -> Result<(), E::Error> {
		log::trace!(target: "afp", "start of process_incoming");
		while let Some(msg) = incoming.try_next().await? {
			if msg.view() != view {
				continue
			}

			log::trace!(target: "afp", "process_incoming: {:?}", msg);
			let SignedMessage { message, signature, id } = msg;
			log.lock().save_message_with_block(id, message, signature);
		}

		log::trace!(target: "afp", "end of process_incoming");
		Ok(())
	}

	/// If this voter is the leader in this view.
	fn is_primary(&self) -> bool {
		self.voter_set.get_primary(self.view) == self.id
	}

	/// PBFT PREPREPARE stage.
	async fn preprepare(&mut self) -> Result<(), E::Error> {
		// This should longer than view_change duration.
		const DELAY_BEFORE_PRE_PREPARE_MILLI: u64 = 1000;
		const DELAY_PRE_PREPARE_MILLI: u64 = 1000;

		log::trace!(target: "afp", "PREPREPARE start.");

		// This sleep is a must.
		Delay::new(Duration::from_millis(DELAY_BEFORE_PRE_PREPARE_MILLI)).await;

		// The PrePrepare is only meaningful if we are primary node.
		//
		// But we still allow other nodes to send preprepare,
		// so that time elapse is consistent.
		let last_round_base = self.message_log.lock().last_round_base.clone();
		// get preprepare hash
		let (height, hash) = self.env.preprepare(self.view, last_round_base.1).await?.unwrap();
		log::trace!(target: "afp", "{:?} preprepare_hash: {:?}", self.id, hash);

		let seq = self.message_log.lock().seq();
		let preprepare = PrePrepare::new(self.view, seq, height, hash);

		self.multicast(Message::PrePrepare(preprepare)).await?;

		if self.message_log.lock().current_state == CurrentState::PrePrepare {
			log::trace!(target: "afp", "PREPREPARE sleep.");
			let log = self.message_log.clone();
			let mut timeout = Delay::new(Duration::from_millis(DELAY_PRE_PREPARE_MILLI));
			let timeout_wait = futures::future::poll_fn(|cx| {
				let mut log = log.lock();
				if log.current_state == CurrentState::PrePrepare {
					log.waker = Some(cx.waker().clone());
					timeout.poll_unpin(cx)
				} else {
					Poll::Ready(())
				}
			});
			timeout_wait.await;
			log::trace!(target: "afp", "PREPREPARE finish sleep.");
			// Delay::new(Duration::from_millis(DELAY_PRE_PREPARE_MILLI)).await;
		}

		log::trace!(target: "afp", "PREPREPARE finish.");
		Ok(())
	}

	/// PBFT PREPARE stage.
	///
	/// Called at end of pre-prepare or beginning of prepare.
	async fn prepare(&mut self) -> Result<(), E::Error> {
		log::trace!(target: "afp", "PREPARE start prepare");
		const DELAY_PREPARE_MILLI: u64 = 1000;

		let (height, hash) = self.message_log.lock().target.clone().unwrap();
		let seq = self.message_log.lock().seq();

		let prepare_msg = Message::Prepare(Prepare::new(self.view, seq, height, hash));

		self.multicast(prepare_msg).await?;

		if self.message_log.lock().current_state == CurrentState::Prepare {
			log::trace!(target: "afp", "PREPARE sleep.");
			let log = self.message_log.clone();
			let mut timeout = Delay::new(Duration::from_millis(DELAY_PREPARE_MILLI));
			let timeout_wait = futures::future::poll_fn(|cx| {
				let mut log = log.lock();
				if log.current_state == CurrentState::Prepare {
					log.waker = Some(cx.waker().clone());
					timeout.poll_unpin(cx)
				} else {
					Poll::Ready(())
				}
			});
			timeout_wait.await;
			log::trace!(target: "afp", "PREPARE finish sleep.");
			// Delay::new(Duration::from_millis(DELAY_PREPARE_MILLI)).await;
		}

		log::trace!(target: "afp", "PREPARE prepare finish.");
		Ok(())
	}

	/// PBFT COMMIT stage.
	async fn commit(&mut self) -> Result<(), E::Error> {
		log::trace!(target: "afp", "COMMIT start commit");
		const DELAY_COMMIT_MILLI: u64 = 1000;

		let (height, hash) = self.message_log.lock().target.clone().unwrap();
		let seq = self.message_log.lock().seq();

		let commit = Commit::new(self.view, seq, height, hash);

		let commit_msg = Message::Commit(commit);

		self.multicast(commit_msg).await?;

		if self.message_log.lock().current_state == CurrentState::Commit {
			log::trace!(target: "afp", "COMMIT sleep.");
			let log = self.message_log.clone();
			let mut timeout = Delay::new(Duration::from_millis(DELAY_COMMIT_MILLI));
			let timeout_wait = futures::future::poll_fn(|cx| {
				let mut log = log.lock();
				if log.current_state == CurrentState::Commit {
					log.waker = Some(cx.waker().clone());
					timeout.poll_unpin(cx)
				} else {
					Poll::Ready(())
				}
			});
			timeout_wait.await;
			log::trace!(target: "afp", "COMMIT finish sleep.");
			// Delay::new(Duration::from_millis(DELAY_COMMIT_MILLI)).await;
		}

		log::trace!(target: "afp", "COMMIT end commit");
		Ok(())
	}

	/// Multicast a message to all peers.
	async fn multicast(&mut self, msg: Message<E::Number, E::Hash>) -> Result<(), E::Error> {
		log::trace!(target: "afp", "{:?} multicast message: {:?}", self.id, msg);
		let result = self.outgoing.as_mut().unwrap().send(msg).await;

		log::trace!(target: "afp", "{:?} multicast message finish with result: {:?}", self.id, result);

		result
	}

	/// Has the primary (leader) sent a message in this round.
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

	/// Voting in a round in PBFT.
	async fn progress(&mut self) -> Result<(E::Number, E::Hash), E::Error> {
		log::trace!(target: "afp", "{:?} ViewRound::progress, view: {}", self.id, self.view);

		// preprepare
		self.preprepare().await?;

		if self.message_log.lock().target.is_none() {
			self.primary_alive()?;
			unreachable!();
		}

		// Stop received messages with the higher seq number.
		self.message_log.lock().block_catch_up();
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

		let f_commit = self.message_log.lock().gen_f_commit();

		if let Some(f_commit) = f_commit {
			log::debug!(target: "afp", "generated a new finalized_commit.");
			// Skip if we already have a same finalized commit.
			// TODO: maybe further check the commit is the same?
			if self.message_log.lock().last_round_base.0.clone() != height {
				log::trace!(target: "afp", "{:?} in view {} finalized a new commit", self.id, self.view);
				let _ = self.env.finalize_block(
					self.view,
					hash.to_owned(),
					height.to_owned(),
					f_commit.to_owned(),
				);

				// After finalize block, we report the progress we've made.
				let last_round_base = self.message_log.lock().last_round_base.to_owned();
				let _ = self.env.complete_f_commit(
					self.view,
					State { finalized: Some((height.to_owned(), hash.to_owned())) },
					last_round_base,
					f_commit,
				);
			} else {
				log::trace!(target: "afp", "skip already finalized block");
			}
		} else {
			return Err(Error::CommitNotEnough.into())
		}

		log::trace!(target: "afp", "{:?} ViewRound::END {:?}", self.id, self.view);

		self.primary_alive()?;

		{
			let mut message_log = self.message_log.lock();
			message_log.bump_seq();
			message_log.last_round_base = message_log.target.clone().unwrap();

			message_log.clear_rounds();
		};

		Ok((height, hash))
	}
}

#[cfg(test)]
mod tests {
	use pretty_assertions::assert_eq;
	// - decode/encode test
	// state change test
	// #[test]
	// fn state_should_change_for_enough_prepare_msg() {
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
	// }
	//
	// // communication test
	#[test]
	fn talking_to_myself() {
		// simple_logger::init_with_level(log::Level::Trace).unwrap();
		let local_id = 5;
		let voter_set = VoterSet::new(vec![5]).unwrap();

		let (network, routing_network) = make_network();
		let global_comms = network.make_global_comms(local_id);

		let env = Arc::new(DummyEnvironment::new(network, local_id));

		// init chain
		let last_finalized = env.with_chain(|chain| {
			chain.push_blocks(GENESIS_HASH, &["A", "B", "C", "D", "E"]);
			log::trace!(
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
					network.make_global_comms(local_id),
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
					network.make_global_comms(local_id),
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
	fn view_did_not_change_when_primary_alive_without_new_preprepare() {
		// simple_logger::init_with_level(log::Level::Trace).unwrap();
		let voters_num = 4;
		let online_voters_num = 4;
		let voter_set = VoterSet::new((0..voters_num).into_iter().collect()).unwrap();

		let (network, mut routing_network) = make_network();
		routing_network.register_global_validator_hook(Box::new(|m| match m {
			super::GlobalMessageIn::CatchUp(_, _) | super::GlobalMessageIn::ViewChange(_) => {
				panic!("should not receive view change or catchup message.");
			},
			_ => {},
		}));
		let mut pool = LocalPool::new();

		let finalized_streams = (0..online_voters_num)
			.map(|local_id| {
				// init chain
				let env = Arc::new(DummyEnvironment::new(network.clone(), local_id));
				let last_finalized = env.with_chain(|chain| {
					chain.push_blocks(GENESIS_HASH, &["A", "B"]);
					chain.last_finalized()
				});

				let finalized = env.finalized_stream();
				let mut voter = Voter::new(
					env.clone(),
					voter_set.clone(),
					network.make_global_comms(local_id),
					1,
					last_finalized,
				);

				pool.spawner()
					.spawn(async move {
						voter.run().await;
					})
					.unwrap();

				future::select(
					Delay::new(Duration::from_secs(20)),
					finalized.for_each(|(_h, n)| {
						assert!(n < 4, "block_number should less than 4.");
						futures::future::ready(())
					}),
				)
			})
			.collect::<Vec<_>>();

		// run voter in background. scheduling it to shut down at the end.
		// futures::executor::block_on(testa(voter));
		pool.spawner().spawn(routing_network).unwrap();

		// wait for the best block to finalized.
		pool.run_until(futures::future::join_all(finalized_streams.into_iter()));
	}

	#[test]
	fn view_change_when_primary_fail() {
		let max_view = Arc::new(Mutex::new(0));
		let max_view_clone = max_view.clone();

		let voters_num = 4;
		let online_voters_num = 4;
		let voter_set = VoterSet::new((0..voters_num).into_iter().collect()).unwrap();

		let (network, mut routing_network) = make_network();

		routing_network.rule.lock().isolate_after(1, 2);
		routing_network.register_global_validator_hook(Box::new(move |m| match m {
			super::GlobalMessageIn::CatchUp(_, _) => {
				panic!("should not receive catchup message.");
			},
			super::GlobalMessageIn::ViewChange(view_change) => {
				let mut max_view = max_view.lock();
				*max_view = view_change.new_view;
			},
			_ => {},
		}));

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
					network.make_global_comms(local_id),
					1,
					last_finalized,
				);

				pool.spawner()
					.spawn(async move {
						let timeout = Delay::new(Duration::from_millis(30000));

						let voter = Box::pin(voter.run());

						futures::future::select(voter, timeout).await;
					})
					.unwrap();

				finalized
					.take_while(move |&(h, n)| {
						log::info!("id: {}, finalized: hash: {}, block_height: {}", local_id, h, n);
						futures::future::ready(n < 6)
					})
					.for_each(|_v| {
						// log::info!("finalized: v: {:?}", v);
						futures::future::ready(())
					})
			})
			.collect::<Vec<_>>();

		// run voter in background. scheduling it to shut down at the end.
		// futures::executor::block_on(testa(voter));
		pool.spawner().spawn(routing_network).unwrap();

		// wait for the best block to finalized.
		pool.run_until(futures::future::join_all(finalized_streams.into_iter()));
		assert_eq!(*max_view_clone.lock(), 2);
	}

	#[test]
	fn fail() {}

	#[test]
	fn skips_to_latest_view_after_catch_up() {
		// simple_logger::init_with_level(log::Level::Trace).unwrap();

		let voters_num = 5;
		let voter_set = VoterSet::new((0..voters_num).into_iter().collect()).unwrap();

		let (network, routing_network) = make_network();
		let mut pool = LocalPool::new();

		pool.spawner().spawn(routing_network).unwrap();

		let (env, mut unsynced_voter) = {
			let local_id = 3;

			let env = Arc::new(DummyEnvironment::new(network.clone(), local_id));
			let last_finalized = env.with_chain(|chain| {
				chain.push_blocks(GENESIS_HASH, &["A", "B", "C", "D", "E"]);
				chain.last_finalized()
			});

			let voter = Voter::new(
				env.clone(),
				voter_set.clone(),
				network.make_global_comms(local_id),
				1,
				last_finalized,
			);

			(env, voter)
		};

		let pp = |id| crate::leader::SignedPrepare {
			id,
			signature: 1234,
			prepare: crate::leader::Prepare { view: 1, seq: 2, target_hash: "C", target_number: 4 },
		};

		let cm = |id| crate::leader::SignedCommit {
			id,
			signature: 1234,
			commit: crate::leader::Commit { view: 1, seq: 2, target_hash: "C", target_number: 4 },
		};

		network.send_message(GlobalMessageIn::CatchUp(
			CatchUp {
				view_number: 3,
				base_number: 1,
				base_hash: GENESIS_HASH,
				prepares: vec![pp(0), pp(1), pp(2), pp(3)],
				commits: vec![cm(0), cm(1), cm(2), cm(3)],
			},
			Callback::Blank,
		));

		let voter_state = unsynced_voter.voter_state();
		assert_eq!(voter_state.get().background_views.get(&2), None);

		// spawn the voter in the background
		pool.spawner().spawn(async move { unsynced_voter.run().await }).unwrap();

		// wait until it's caught up, it should skip to round 6 and send a
		// finality notification for the block that was finalized by catching
		// up.
		let caught_up = future::poll_fn(|_| {
			log::info!("polling");
			if voter_state.get().best_view.0 == 3 {
				Poll::Ready(())
			} else {
				Poll::Pending
			}
		});

		let finalized = env.finalized_stream().take(1).for_each(|(h, n)| {
			log::info!("finalized: hash: {}, block_height: {}", h, n);
			Delay::new(Duration::from_millis(1100))
		});

		pool.run_until(caught_up.then(|_| finalized));

        let mut best_view = voter_state.get().best_view.clone();
        best_view.1.prepare_ids = Default::default();
		assert_eq!(
			best_view,
			(
				3,
				crate::leader::voter::report::ViewState::<Hash, Id> {
					state: crate::leader::CurrentState::Prepare,
					total_voters: 5,
					threshold: 4,
					preprepare_hash: Some("D"),
					prepare_ids: Default::default(),
					commit_ids: Default::default(),
				}
			)
		);

		// assert_eq!(
		// 	voter_state.get().background_rounds.get(&5),
		// 	Some(&report::RoundState::<Id> {
		// 		total_weight,
		// 		threshold_weight,
		// 		prevote_current_weight: VoteWeight(3),
		// 		prevote_ids: voter_ids.clone(),
		// 		precommit_current_weight: VoteWeight(3),
		// 		precommit_ids: voter_ids,
		// 	})
		// );
	}

	// TODO: This cannot be tested via unit test.
	fn fail_then_recover() {
		let max_view = Arc::new(Mutex::new(0));
		let max_view_clone = max_view.clone();

		let voters_num = 4;
		let online_voters_num = 4;
		let voter_set = VoterSet::new((0..voters_num).into_iter().collect()).unwrap();

		let _fail_then_recover =
			move |from: &Id, from_state: &VoterState, to: &Id, to_state: &VoterState| {
				let node = 1;
				let after = 2;
				let recover = 4;

				if (from == to) && (from == &node) {
					true
				} else if (from != &node && from_state.last_finalized >= recover) ||
					(to == &node && to_state.last_finalized >= recover)
				{
					true
				} else {
					!((from == &node && from_state.last_finalized >= after) ||
						(to == &node && to_state.last_finalized >= after))
				}
			};

		let (network, mut routing_network) = make_network();

		routing_network.rule.lock().add_rule(Box::new(_fail_then_recover));

		routing_network.register_global_validator_hook(Box::new(move |m| match m {
			super::GlobalMessageIn::CatchUp(_, _) => {
				panic!("should not receive catchup message.");
			},
			super::GlobalMessageIn::ViewChange(view_change) => {
				let mut max_view = max_view.lock();
				*max_view = view_change.new_view;
			},
			_ => {},
		}));

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
					network.make_global_comms(local_id),
					1,
					last_finalized,
				);

				pool.spawner()
					.spawn(async move {
						voter.run().await;
					})
					.unwrap();

				finalized
					.take_while(move |&(h, n)| {
						log::info!("id: {}, finalized: hash: {}, block_height: {}", local_id, h, n);
						futures::future::ready(n < 6)
					})
					.for_each(|_v| {
						// log::info!("finalized: v: {:?}", v);
						futures::future::ready(())
					})
			})
			.collect::<Vec<_>>();

		// run voter in background. scheduling it to shut down at the end.
		// futures::executor::block_on(testa(voter));
		pool.spawner().spawn(routing_network).unwrap();

		// wait for the best block to finalized.
		pool.run_until(futures::future::join_all(finalized_streams.into_iter()));
		assert_eq!(*max_view_clone.lock(), 2);
	}

	use std::{sync::Arc, task::Poll, time::Duration};

	use futures::{executor::LocalPool, future, task::SpawnExt, FutureExt, StreamExt};
	use futures_timer::Delay;
	use parking_lot::Mutex;

	use crate::leader::{
		testing::{
			chain::GENESIS_HASH,
			environment::{make_network, DummyEnvironment, VoterState},
			Hash, Id,
		},
		CatchUp, VoterSet,
	};

	use super::{Callback, GlobalMessageIn, Voter};
}
