//! pbft finality gadget.
//!
#![allow(missing_docs)]
#![allow(dead_code)]
#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

use std::num::NonZeroUsize;

#[cfg(feature = "derive-codec")]
use parity_scale_codec::{Decode, Encode};
#[cfg(feature = "derive-codec")]
use scale_info::TypeInfo;

#[cfg(not(feature = "std"))]
mod std {
	pub use core::{cmp, hash, iter, mem, num, ops};

	pub mod vec {
		pub use alloc::vec::Vec;
	}

	pub mod collections {
		pub use alloc::collections::{
			btree_map::{self, BTreeMap},
			btree_set::{self, BTreeSet},
		};
	}

	pub mod fmt {
		pub use core::fmt::{Display, Formatter, Result};

		pub trait Debug {}
		impl<T> Debug for T {}
	}
}

use crate::std::{collections::BTreeMap, vec::Vec};

/// Error for PBFT consensus.
#[derive(Debug, Clone)]
pub enum Error {
	// incase of primary failure, need to view change.
	// NoPrePrepare,
	/// No Primary message was received.
	PrimaryFailure,
	CommitNotEnough,
	IncomingClosed,
}

#[cfg(feature = "std")]
pub mod voter;

#[cfg(all(test, feature = "std"))]
mod testing;

impl std::fmt::Display for Error {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		match *self {
			Error::PrimaryFailure => {
				write!(f, "No primary message, may need to initiate view change.")
			},
			Error::IncomingClosed => {
				write!(f, "Incoming channel is closed (unexpected).")
			},
			Error::CommitNotEnough => {
				write!(f, "Commit not enough.")
			},
		}
	}
}

pub mod view_round {
	#[cfg(feature = "derive-codec")]
	use parity_scale_codec::{Decode, Encode};
	#[cfg(feature = "derive-codec")]
	use scale_info::TypeInfo;

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
			State { finalized: Some(genesis.clone()), completable: true }
		}
	}
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}

/// A preprepare message for a block in PBFT.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "derive-codec", derive(Encode, Decode, TypeInfo))]
pub struct PrePrepare<N, D> {
	pub view: u64,
	pub seq: u64,
	pub target_number: N,
	pub target_hash: D,
}

impl<N, D> PrePrepare<N, D> {
	/// Create a new preprepare message.
	pub fn new(view: u64, seq: u64, target_number: N, target_hash: D) -> Self {
		PrePrepare { view, seq, target_number, target_hash }
	}
}

/// A signed preprepare message.
#[derive(Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "std", test), derive(Debug))]
#[cfg_attr(feature = "derive-codec", derive(Encode, Decode, TypeInfo))]
pub struct SignedPrePrepare<N, D, S, Id> {
	/// The preprepare message which has been signed.
	pub preprepare: PrePrepare<N, D>,
	/// The signature on the message.
	pub signature: S,
	/// The Id of the signer.
	pub id: Id,
}

/// A prepare message for a block in PBFT.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "derive-codec", derive(Encode, Decode, TypeInfo))]
pub struct Prepare<N, D> {
	/// The view number.
	pub view: u64,
	pub seq: u64,
	/// The target block's number.
	pub target_number: N,
	/// The target block's hash.
	pub target_hash: D,
}

impl<N, D> Prepare<N, D> {
	/// Create a new prepare message.
	pub fn new(view: u64, seq: u64, target_number: N, digest: D) -> Self {
		Prepare { view, seq, target_number, target_hash: digest }
	}
}

/// A signed prepare message.
#[derive(Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "std", test), derive(Debug))]
#[cfg_attr(feature = "derive-codec", derive(Encode, Decode, TypeInfo))]
pub struct SignedPrepare<N, D, S, Id> {
	/// The prepare message which has been signed.
	pub prepare: Prepare<N, D>,
	/// The signature on the message.
	pub signature: S,
	/// The Id of the signer.
	pub id: Id,
}

/// A commit message for a block in PBFT.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "derive-codec", derive(Encode, Decode, TypeInfo))]
pub struct Commit<N, D> {
	/// The view number.
	pub view: u64,
	pub seq: u64,
	/// The sequence number.
	pub target_number: N,
	/// The target block's hash.
	pub target_hash: D,
}

impl<N, D> Commit<N, D> {
	/// Create a new commit message.
	pub fn new(view: u64, seq: u64, target_number: N, target_hash: D) -> Self {
		Commit { view, seq, target_number, target_hash }
	}
}

/// A signed commit message.
#[derive(Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "std", test), derive(Debug))]
#[cfg_attr(feature = "derive-codec", derive(Encode, Decode, TypeInfo))]
pub struct SignedCommit<N, D, S, Id> {
	/// The commit message which has been signed.
	pub commit: Commit<N, D>,
	/// The signature on the message.
	pub signature: S,
	/// The Id of the signer.
	pub id: Id,
}

/// N: sequence number
/// D: Digest of the block, that is header
#[derive(Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "std", test), derive(Debug))]
#[cfg_attr(feature = "derive-codec", derive(Encode, Decode, TypeInfo))]
pub enum Message<N, D> {
	/// A message to be sent to the network by primary alive but not yet haprimary alive but not
	/// yet have a new block to make consensus.
	PrePrepare(PrePrepare<N, D>),
	/// multicast (except from the primary) <view, number, Digest(m)>
	Prepare(Prepare<N, D>),
	/// multicast <view, number>
	Commit(Commit<N, D>),
}

impl<D, N: Copy> Message<N, D> {
	/// Get the target block of the vote.
	pub fn target(&self) -> (&D, N) {
		match *self {
			Message::PrePrepare(ref v) => (&v.target_hash, v.target_number),
			Message::Prepare(ref v) => (&v.target_hash, v.target_number),
			Message::Commit(ref v) => (&v.target_hash, v.target_number),
		}
	}

	pub fn view(&self) -> u64 {
		match *self {
			Message::PrePrepare(ref v) => v.view,
			Message::Prepare(ref v) => v.view,
			Message::Commit(ref v) => v.view,
		}
	}

	pub fn seq(&self) -> u64 {
		match *self {
			Message::PrePrepare(ref v) => v.seq,
			Message::Prepare(ref v) => v.seq,
			Message::Commit(ref v) => v.seq,
		}
	}
}

/// A commit message which is an aggregate of commits.
/// NOTE: Similar to `Commit` in GRANDPA.
#[derive(PartialEq, Eq, Clone)]
#[cfg_attr(any(feature = "std", test), derive(Debug))]
#[cfg_attr(feature = "derive-codec", derive(Encode, Decode, TypeInfo))]
pub struct FinalizedCommit<N, D, S, Id> {
	/// The target block's hash.
	pub target_hash: D,
	/// The target block's number.
	pub target_number: N,
	/// Precommits for target block or any block after it that justify this commit.
	pub commits: Vec<SignedCommit<N, D, S, Id>>,
}

/// A signed message.
#[derive(Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "std", test), derive(Debug))]
#[cfg_attr(feature = "derive-codec", derive(Encode, Decode, TypeInfo))]
pub struct SignedMessage<N, D, S, Id> {
	/// The internal message which has been signed.
	pub message: Message<N, D>,
	/// The signature on the message.
	pub signature: S,
	/// The Id of the signer
	pub id: Id,
}

impl<N: Copy, D, S, Id> SignedMessage<N, D, S, Id> {
	/// Get the target block of the vote.
	pub fn target(&self) -> (&D, N) {
		self.message.target()
	}

	pub fn view(&self) -> u64 {
		self.message.view()
	}

	pub fn seq(&self) -> u64 {
		self.message.seq()
	}
}

/// Authentication data for a set of many messages, currently a set of precommit signatures but
/// in the future could be optimized with BLS signature aggregation.
pub type MultiAuthData<S, Id> = Vec<(S, Id)>;

/// A commit message with compact representation of authenticationg data.
/// NOTE: Similar to `CompactCommit`
#[derive(Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "std", test), derive(Debug))]
#[cfg_attr(feature = "derive-codec", derive(Encode, Decode, TypeInfo))]
pub struct CompactCommit<D, N, S, Id> {
	/// The target block's hash.
	pub target_hash: D,
	/// The target block's number.
	pub target_number: N,
	/// Precommits for target block or any block after it that justify this commit.
	pub commits: Vec<Commit<N, D>>,
	/// Authentication data for the commit.
	pub auth_data: MultiAuthData<S, Id>,
}

impl<D: Clone, N: Clone, S, Id> From<FinalizedCommit<N, D, S, Id>> for CompactCommit<D, N, S, Id> {
	fn from(commit: FinalizedCommit<N, D, S, Id>) -> Self {
		CompactCommit {
			target_hash: commit.target_hash,
			target_number: commit.target_number,
			commits: commit.commits.iter().map(|c| c.commit.clone()).collect(),
			auth_data: commit.commits.into_iter().map(|c| (c.signature, c.id)).collect(),
		}
	}
}

impl<D, N, S, Id> From<CompactCommit<D, N, S, Id>> for FinalizedCommit<N, D, S, Id> {
	fn from(commit: CompactCommit<D, N, S, Id>) -> Self {
		FinalizedCommit {
			target_hash: commit.target_hash,
			target_number: commit.target_number,
			commits: commit
				.commits
				.into_iter()
				.zip(commit.auth_data.into_iter())
				.map(|(c, (s, id))| SignedCommit { commit: c, signature: s, id })
				.collect(),
		}
	}
}

/// A catch-up message, which is an aggregate of prevotes and precommits necessary
/// to complete a round.
///
/// This message contains a "base", which is a block all of the vote-targets are
/// a descendent of.
#[derive(Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "std", test), derive(Debug))]
#[cfg_attr(feature = "derive-codec", derive(Encode, Decode, TypeInfo))]
pub struct CatchUp<N, D, S, Id> {
	/// View number.
	pub view_number: u64,
	/// Prevotes for target block or any block after it that justify this catch-up.
	pub prepares: Vec<SignedPrepare<N, D, S, Id>>,
	/// Precommits for target block or any block after it that justify this catch-up.
	pub commits: Vec<SignedCommit<N, D, S, Id>>,
	/// The base hash. See struct docs.
	pub base_hash: D,
	/// The base number. See struct docs.
	pub base_number: N,
}

#[derive(Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "std", test), derive(Debug))]
#[cfg_attr(feature = "derive-codec", derive(Encode, Decode, TypeInfo))]
pub struct ViewChange<Id> {
	pub new_view: u64,
	pub id: Id,
}

impl<Id> ViewChange<Id> {
	pub fn new(new_view: u64, id: Id) -> Self {
		Self { new_view, id }
	}
}

#[cfg_attr(any(feature = "std", test), derive(Debug))]
#[derive(PartialEq, Eq, Clone, Copy)]
pub enum CurrentState {
	// Initial state.
	PrePrepare,
	Prepare,
	Commit,
}

/// Arithmetic necessary for a block number.
pub trait BlockNumberOps:
	std::fmt::Debug
	+ std::cmp::Ord
	+ std::ops::Add<Output = Self>
	+ std::ops::Sub<Output = Self>
	+ num::One
	+ num::Zero
	+ num::AsPrimitive<usize>
{
}

impl<T> BlockNumberOps for T
where
	T: std::fmt::Debug,
	T: std::cmp::Ord,
	T: std::ops::Add<Output = Self>,
	T: std::ops::Sub<Output = Self>,
	T: num::One,
	T: num::Zero,
	T: num::AsPrimitive<usize>,
{
}

/// similar to: [`round::Round`]
#[cfg_attr(any(feature = "std", test), derive(Debug))]
pub(crate) struct Storage<N, D, S, Id> {
	seq: u64,
	last_round_base: (N, D),
	current_state: CurrentState,
	// from valid preprepare msg.
	target: Option<(N, D)>,
	preprepare: BTreeMap<Id, (PrePrepare<N, D>, S)>,
	prepare: BTreeMap<Id, (Prepare<N, D>, S)>,
	commit: BTreeMap<Id, (Commit<N, D>, S)>,
}

/// State of the view. Generate by [`Storage`].
#[derive(PartialEq, Clone)]
#[cfg_attr(any(feature = "std", test), derive(Debug))]
#[cfg_attr(feature = "derive-codec", derive(Encode, Decode, scale_info::TypeInfo))]
pub struct State<N, H> {
	/// The last finalized block.
	pub finalized: Option<(N, H)>,
	pub completable: bool,
}

impl<H: Clone, N: Clone> State<H, N> {
	/// Genesis state.
	pub fn genesis(genesis: (H, N)) -> Self {
		State { finalized: Some(genesis.clone()), completable: true }
	}
}

impl<N, H, Id, S> Storage<N, H, S, Id>
where
	Id: Clone + Eq + std::hash::Hash + Ord + std::fmt::Debug,
	H: std::fmt::Debug,
	N: std::fmt::Debug,
{
	fn new(last_round_base: (N, H)) -> Self {
		Self {
			seq: 0,
			last_round_base,
			current_state: CurrentState::PrePrepare,
			target: None,
			preprepare: Default::default(),
			prepare: Default::default(),
			commit: Default::default(),
		}
	}

	fn contains_key(&self, key: &Id) -> bool {
		self.preprepare.contains_key(key) ||
			self.prepare.contains_key(key) ||
			self.commit.contains_key(key)
	}

	fn seq(&self) -> u64 {
		self.seq
	}

	fn bump_seq(&mut self) {
		self.seq += 1;
	}

	fn clear_votes(&mut self) {
		self.preprepare.clear();
		self.prepare.clear();
		self.commit.clear();
	}
	// Return state of the view.
	// TODO: maybe used in testing RotingRule.
	// pub fn state(&self) -> State<H, N> {
	// 	let mut completable = true;
	// 	if self.current_state == CurrentState::PrePrepare {
	// 		completable = false;
	// 	}
	// 	State { finalized: self.finalized(), completable }
	// }
}

impl<N, H, Id: Eq + Ord + std::hash::Hash, S> Storage<N, H, S, Id>
where
	Id: std::fmt::Debug + Clone,
	H: std::fmt::Debug + Clone + std::cmp::PartialEq,
	N: std::fmt::Debug + Clone + std::cmp::PartialEq + std::cmp::PartialOrd + Copy,
	S: std::fmt::Debug,
{
	fn target(&self) -> Option<(&H, N)> {
		self.target.as_ref().map(|(n, h)| (h, *n))
	}

	fn save_message(&mut self, from: Id, message: Message<N, H>, signature: S) {
		if message.seq() != self.seq() || self.target().map_or(false, |t| t != message.target()) {
			return
		}

		match message {
			Message::Prepare(msg) => {
				// if let Some(target) = self.target.clone() {
				// 	if msg.target_number != target.0 && msg.target_hash != target.1 {
				// 		return
				// 	}
				// }
				#[cfg(feature = "std")]
				log::trace!(target: "afp", "insert message to Prepare, msg: {:?}", msg);
				self.prepare.insert(from, (msg, signature));
			},
			Message::Commit(msg) => {
				// if let Some(target) = self.target.clone() {
				// 	if msg.target_number != target.0 && msg.target_hash != target.1 {
				// 		return
				// 	}
				// }
				#[cfg(feature = "std")]
				log::trace!(target: "afp", "insert message to Commit, msg: {:?}", msg);
				self.commit.insert(from, (msg, signature));
			},
			Message::PrePrepare(msg) => {
				if msg.target_number < self.last_round_base.0 {
					return
				}
				#[cfg(feature = "std")]
				log::trace!(target: "afp", "insert message to preprepare, msg: EmptyPrePrepare");
				self.preprepare.insert(from, (msg, signature));
			},
		}
	}

	#[cfg(feature = "std")]
	fn print_log(&self) {
		log::trace!(target: "afp", "pre-prepare: {:?}", self.target);
		for i in self.prepare.iter() {
			log::trace!(target: "afp", "  {:?}", i);
		}
		for i in self.commit.iter() {
			log::trace!(target: "afp", "  {:?}", i);
		}
		log::trace!(target: "afp", "=== end ===")
	}

	fn count_prepares(&self) -> usize {
		self.prepare.len()
	}

	fn count_commits(&self) -> usize {
		self.commit.len()
	}
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct VoterSet<Id: Eq + Ord> {
	voters: Vec<Id>,
	/// The required number threshold for supermajority.
	/// Normally, it's > 2/3.
	threshold: usize,
}

impl<Id: Eq + Ord + Clone> VoterSet<Id> {
	pub fn new(voters: Vec<Id>) -> Option<Self> {
		if voters.is_empty() {
			None
		} else {
			let len = voters.len() - (voters.len() - 1) / 3;
			Some(Self { voters, threshold: len })
		}
	}

	pub fn add(&mut self, id: Id) {
		self.voters.push(id);
	}

	pub fn remove(&mut self, id: &Id) {
		self.voters.retain(|x| x != id);
	}

	pub fn is_empty(&self) -> bool {
		self.voters.is_empty()
	}

	pub fn is_full(&self) -> bool {
		self.voters.len() >= self.threshold
	}

	pub fn is_member(&self, id: &Id) -> bool {
		self.voters.contains(id)
	}

	pub fn threshold(&self) -> usize {
		self.threshold
	}

	/// Get the size of the set.
	pub fn len(&self) -> NonZeroUsize {
		unsafe {
			// SAFETY: By VoterSet::new()
			NonZeroUsize::new_unchecked(self.voters.len())
		}
	}

	/// Get the nth voter in the set, if any.
	///
	/// Returns `None` if `n >= len`.
	pub fn nth(&self, n: usize) -> Option<&Id> {
		self.voters.get(n)
	}

	pub fn voters(&self) -> &[Id] {
		&self.voters
	}

	pub fn get_primary(&self, view: u64) -> Id {
		self.voters.get(view as usize % self.voters.len()).cloned().unwrap()
	}

	/// Whether the set contains a voter with the given ID.
	pub fn contains(&self, id: &Id) -> bool {
		self.voters.binary_search_by_key(&id, |id| id).is_ok()
	}

	/// Get an iterator over the voters in the set, as given by
	/// the associated total order.
	pub fn iter(&self) -> impl Iterator<Item = &Id> {
		self.voters.iter()
	}

	/// Get the voter info for the voter with the given ID, if any.
	pub fn get(&self, id: &Id) -> Option<&Id> {
		self.voters.binary_search_by_key(&id, |id| id).ok().map(|idx| &self.voters[idx])
	}
}

/// Struct returned from `validate_commit` function with information
/// about the validation result.
pub struct CommitValidationResult<H, N> {
	target: Option<(H, N)>,
	num_commits: usize,
	num_duplicated_commits: usize,
	num_invalid_voters: usize,
}

impl<H, N> CommitValidationResult<H, N> {
	pub fn target(&self) -> Option<&(H, N)> {
		self.target.as_ref()
	}

	/// Returns the number of precommits in the commit.
	pub fn num_commits(&self) -> usize {
		self.num_commits
	}

	/// Returns the number of duplicate precommits in the commit.
	pub fn num_duplicated_commits(&self) -> usize {
		self.num_duplicated_commits
	}

	/// Returns the number of invalid voters in the commit.
	pub fn num_invalid_voters(&self) -> usize {
		self.num_invalid_voters
	}
}

impl<H, N> Default for CommitValidationResult<H, N> {
	fn default() -> Self {
		CommitValidationResult {
			target: None,
			num_commits: 0,
			num_duplicated_commits: 0,
			num_invalid_voters: 0,
		}
	}
}

#[cfg(test)]
mod test {
	use super::*;

	#[test]
	fn test_threshold() {
		assert_eq!(VoterSet::new((0..1).into_iter().collect()).unwrap().threshold, 1);
		assert_eq!(VoterSet::new((0..2).into_iter().collect()).unwrap().threshold, 2);
		assert_eq!(VoterSet::new((0..3).into_iter().collect()).unwrap().threshold, 3);
		assert_eq!(VoterSet::new((0..4).into_iter().collect()).unwrap().threshold, 3);
		assert_eq!(VoterSet::new((0..5).into_iter().collect()).unwrap().threshold, 4);
	}
}
