use std::{sync::Arc, time::Duration};

pub(crate) mod helper {
	// TODO: eliminate this
	pub fn supermajority(n: usize, valid_number: usize) -> bool {
		n > 3 * (n - valid_number)
	}
}

pub struct Voter<E: Environment, GlobalIn, GlobalOut> {
	local_id: E::Id,
	env: Arc<E>,
	voters: VoterSet<E::Id>,
	global_incoming: GlobalIn,
	global_outgoing: GlobalOut,
	// TODO: consider wrap this around sth.
	inner: Option<(ViewRound<E>, E::In)>,
	last_finalized_number: E::Number,
	current_view_number: u64,
}

impl<E: Environment, GlobalIn, GlobalOut> Voter<E, GlobalIn, GlobalOut>
where
	GlobalIn: Stream<Item = Result<GlobalMessage<E::Id>, E::Error>> + Unpin,
	GlobalOut: Sink<GlobalMessage<E::Id>, Error = E::Error> + Unpin,
{
	pub fn new(
		env: Arc<E>,
		voters: VoterSet<E::Id>,
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
		id: E::Id,
		global_outgoing: &mut GlobalOut,
		global_incoming: &mut GlobalIn,
		voters: &VoterSet<E::Id>,
	) -> Result<u64, E::Error> {
		const DELAY_VIEW_CHANGE_WAIT: u64 = 1000;
		let mut new_view = current_view + 1;
		loop {
			log::info!("id: {:?}, start view_change: {new_view}", id);
			// create new view change message
			let view_change = GlobalMessage::ViewChange { new_view, id: id.clone() };
			let mut log = HashMap::<E::Id, GlobalMessage<E::Id>>::new();

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
	pub struct RoundData<Id, Timer, Input, Output> {
		/// Local voter id
		pub voter_id: Id,
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
	use super::super::CurrentState;
	use std::collections::{HashMap, HashSet};

	#[derive(Clone, Debug)]
	pub struct ViewState<Hash, Id> {
		pub state: CurrentState,
		pub total_number: usize,
		pub threshold: usize,
		pub preprepare_hash: Option<Hash>,
		pub prepare_count: usize,
		/// The identities of nodes that have cast prepare so far.
		pub prepare_ids: HashSet<Id>,
		pub commit_count: usize,
		pub commit_ids: HashSet<Id>,
	}

	#[derive(Clone, Debug)]
	pub struct VoterState<Hash, Id> {
		/// Voting rounds running in the background.
		pub background_views: HashMap<u64, ViewState<Hash, Id>>,
		/// The current best voting view.
		pub best_view: (u64, ViewState<Hash, Id>),
	}

	#[derive(Clone, Debug)]
	pub struct Log<Hash, Id> {
		pub id: Id,
		pub state: ViewState<Hash, Id>,
	}
}

/// Environment for consensus.
///
/// Including network, storage and other info (that cannot get directly via consensus algo).
use std::collections::HashMap;

use futures::{Future, FutureExt, Sink, SinkExt, Stream, TryStreamExt};
use futures_timer::Delay;
use parking_lot::Mutex;

use crate::BlockNumberOps;

use super::{CurrentState, Error, GlobalMessage, Message, SignedMessage, Storage, VoterSet};

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
	type Error: From<Error> + ::std::error::Error + Clone;
	/// Hash type used in blockchain or digest.
	type Hash: Eq + Clone + core::fmt::Debug;
	/// The block number type.
	type Number: BlockNumberOps;

	/// Get Voter data.
	fn voter_data(&self) -> communicate::VoterData<Self::Id>;

	/// Get round data.
	fn round_data(
		&self,
		view: u64,
	) -> communicate::RoundData<Self::Id, Self::Timer, Self::In, Self::Out>;

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
	current_state: CurrentState,
	message_log: Arc<Mutex<Storage<E::Number, E::Hash, E::Id>>>,
	view: u64,
	current_height: E::Number,
	id: E::Id,
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
		voter_set: VoterSet<E::Id>,
		env: Arc<E>,
		voter_id: E::Id,
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
		log: Arc<Mutex<Storage<E::Number, E::Hash, E::Id>>>,
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

	use std::sync::Arc;

	use futures::{executor::LocalPool, task::SpawnExt, StreamExt};

	use crate::leader::{
		testing::environment::make_network,
		testing::{chain::GENESIS_HASH, environment::DummyEnvironment},
		VoterSet,
	};

	use super::Voter;
}