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

#[cfg_attr(any(feature = "std", test), derive(Debug))]
#[derive(Clone)]
pub struct SignedMessage<N, H, Signature, ID> {
	from: ID,
	message: Message<N, H>,
	signature: Signature,
}

#[cfg_attr(any(feature = "std", test), derive(Debug))]
#[derive(PartialEq, Clone)]
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
#[cfg_attr(any(feature = "std", test), derive(Debug))]
struct Storage<N, H, ID> {
	preprepare_hash: Option<H>,
	preprepare: BTreeMap<ID, ()>,
	prepare: BTreeMap<ID, Message<N, H>>,
	commit: BTreeMap<ID, Message<N, H>>,
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

impl<N, H, ID: Eq + Ord + std::hash::Hash> Storage<N, H, ID>
where
	ID: std::fmt::Debug,
	H: std::fmt::Debug,
	N: std::fmt::Debug,
{
	fn save_message(&mut self, from: ID, message: Message<N, H>) {
		#[cfg(feature = "std")]
		log::trace!("insert message to Storage, from: {:?}", from);
		match message {
			msg @ Message::Prepare { .. } => {
				#[cfg(feature = "std")]
				log::trace!("insert message to Prepare, msg: {:?}", msg);
				self.prepare.insert(from, msg);
			},
			msg @ Message::Commit { .. } => {
				#[cfg(feature = "std")]
				log::trace!("insert message to Commit, msg: {:?}", msg);
				self.commit.insert(from, msg);
			},
			Message::EmptyPrePrepare => {
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
