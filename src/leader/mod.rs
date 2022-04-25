//! pbft finality gadget.
//!
#![allow(missing_docs)]
#![allow(dead_code)]
#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

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
}

#[cfg(feature = "std")]
pub mod voter;

#[cfg(any(test))]
mod testing;

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
pub struct PrePrepare {}

impl PrePrepare {
	/// Create a new preprepare message.
	pub fn new() -> Self {
		PrePrepare {}
	}
}

/// A signed preprepare message.
#[derive(Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "std", test), derive(Debug))]
#[cfg_attr(feature = "derive-codec", derive(Encode, Decode, TypeInfo))]
pub struct SignedPrePrepare<S, Id> {
	/// The preprepare message which has been signed.
	pub preprepare: PrePrepare,
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
	view: u64,
	/// The sequence number.
	seq_number: N,
	/// The target block's hash.
	digest: D,
}

impl<N, D> Prepare<N, D> {
	/// Create a new prepare message.
	pub fn new(view: u64, seq_number: N, digest: D) -> Self {
		Prepare { view, seq_number, digest }
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
pub struct Commit<N> {
	/// The view number.
	view: u64,
	/// The sequence number.
	seq_number: N,
}

impl<N> Commit<N> {
	/// Create a new commit message.
	pub fn new(view: u64, seq_number: N) -> Self {
		Commit { view, seq_number }
	}
}

/// A signed commit message.
#[derive(Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "std", test), derive(Debug))]
#[cfg_attr(feature = "derive-codec", derive(Encode, Decode, TypeInfo))]
pub struct SignedCommit<N, S, Id> {
	/// The commit message which has been signed.
	pub commit: Commit<N>,
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
	PrePrepare(PrePrepare),
	/// multicast (except from the primary) <view, number, Digest(m)>
	Prepare(Prepare<N, D>),
	/// multicast <view, number>
	Commit(Commit<N>),
}

impl<H, N: Copy> Message<H, N> {
	// TODO: get target?
}

/// A commit message which is an aggregate of commits.
/// NOTE: Similar to `Commit` in GRANDPA.
pub struct FinalizedCommit {}

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

/// Authentication data for a set of many messages, currently a set of precommit signatures but
/// in the future could be optimized with BLS signature aggregation.
pub type MultiAuthData<S, Id> = Vec<(S, Id)>;

/// A commit message with compact representation of authenticationg data.
/// NOTE: Similar to `CompactCommit`
#[derive(Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "std", test), derive(Debug))]
#[cfg_attr(feature = "derive-codec", derive(Encode, Decode, TypeInfo))]
pub struct CompactCommit<H, N, S, Id> {
	/// The target block's hash.
	pub target_hash: H,
	/// The target block's number.
	pub target_number: N,
	/// Precommits for target block or any block after it that justify this commit.
	pub commits: Vec<Commit<N>>,
	/// Authentication data for the commit.
	pub auth_data: MultiAuthData<S, Id>,
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
	pub commits: Vec<SignedCommit<N, S, Id>>,
	/// The base hash. See struct docs.
	pub base_hash: D,
	/// The base number. See struct docs.
	pub base_number: N,
}

#[derive(Clone, PartialEq, Eq)]
#[cfg_attr(any(feature = "std", test), derive(Debug))]
#[cfg_attr(feature = "derive-codec", derive(Encode, Decode, TypeInfo))]
pub struct ViewChange<Id> {
	new_view: u64,
	id: Id,
}

impl<Id> ViewChange<Id> {
	pub fn new(new_view: u64, id: Id) -> Self {
		Self { new_view, id }
	}
}

#[cfg_attr(any(feature = "std", test), derive(Debug))]
#[derive(PartialEq, Eq, Clone)]
pub enum CurrentState {
	// Initial state.
	PrePrepare,
	Prepare,
	Commit,
	// ChangeView,
	// ViewChangeAck,
	// NewView,
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
struct Storage<N, H, Id> {
	preprepare_hash: Option<H>,
	preprepare: BTreeMap<Id, ()>,
	prepare: BTreeMap<Id, Prepare<N, H>>,
	commit: BTreeMap<Id, Commit<N>>,
}

/// State of the view. Generate by [`Storage`].
#[derive(PartialEq, Clone)]
#[cfg_attr(any(feature = "std", test), derive(Debug))]
#[cfg_attr(feature = "derive-codec", derive(Encode, Decode, scale_info::TypeInfo))]
pub struct State<H, N> {
	/// The finalized block.
	pub finalized: Option<(H, N)>,
	/// Whether the round is completable.
	pub completable: bool,
}

impl<H: Clone, N: Clone> State<H, N> {
	/// Genesis state.
	pub fn genesis(genesis: (H, N)) -> Self {
		State { finalized: Some(genesis.clone()), completable: true }
	}
}

impl<N, H, Id> Storage<N, H, Id>
where
	Id: Clone + Eq + std::hash::Hash + Ord + std::fmt::Debug,
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

	fn contains_key(&self, key: &Id) -> bool {
		self.preprepare.contains_key(key)
			|| self.prepare.contains_key(key)
			|| self.commit.contains_key(key)
	}
}

impl<N, H, Id: Eq + Ord + std::hash::Hash> Storage<N, H, Id>
where
	Id: std::fmt::Debug,
	H: std::fmt::Debug,
	N: std::fmt::Debug,
{
	fn save_message(&mut self, from: Id, message: Message<N, H>) {
		#[cfg(feature = "std")]
		log::trace!("insert message to Storage, from: {:?}", from);
		match message {
			Message::Prepare(msg) => {
				#[cfg(feature = "std")]
				log::trace!("insert message to Prepare, msg: {:?}", msg);
				self.prepare.insert(from, msg);
			},
			Message::Commit(msg) => {
				#[cfg(feature = "std")]
				log::trace!("insert message to Commit, msg: {:?}", msg);
				self.commit.insert(from, msg);
			},
			Message::PrePrepare(..) => {
				#[cfg(feature = "std")]
				log::trace!("insert message to preprepare, msg: EmptyPrePrepare");
				self.preprepare.insert(from, ());
			},
		}
	}

	#[cfg(feature = "std")]
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
pub struct VoterSet<Id: Eq + Ord> {
	voters: Vec<Id>,
	/// The required number threshould for supermajority.
	/// Normally, it's > 2/3.
	threshould: usize,
}

impl<Id: Eq + Ord + Clone> VoterSet<Id> {
	pub fn new(voters: Vec<Id>) -> Self {
		let len = voters.len() / 3 + 1;
		Self { voters, threshould: len }
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
		self.voters.len() >= self.threshould
	}

	pub fn is_member(&self, id: &Id) -> bool {
		self.voters.contains(id)
	}

	pub fn len(&self) -> usize {
		self.voters.len()
	}

	pub fn threshould(&self) -> usize {
		self.threshould
	}

	pub fn voters(&self) -> &[Id] {
		&self.voters
	}

	pub fn get_primary(&self, view: u64) -> Id {
		self.voters.get(view as usize % self.voters.len()).cloned().unwrap()
	}

	/// Get an iterator over the voters in the set, as given by
	/// the associated total order.
	pub fn iter(&self) -> impl Iterator<Item = &Id> {
		self.voters.iter()
	}
}
