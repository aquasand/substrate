// This file is part of Substrate.

// Copyright (C) 2019-2021 Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// 	http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Traits for dealing with multiple collections of non-fungible assets.
//!
//! This assumes a dual-level namespace identified by `Inspect::InstanceId`, and could
//! reasonably be implemented by pallets which want to expose multiple independent collections of
//! NFT-like objects.
//!
//! For an NFT API which has single-level namespacing, the traits in `nonfungible` are better to
//! use.
//!
//! Implementations of these traits may be converted to implementations of corresponding
//! `nonfungible` traits by using the `nonfungible::ItemOf` type adapter.

use sp_std::prelude::*;
use codec::{Encode, Decode};
use sp_runtime::TokenError;
use crate::dispatch::DispatchResult;

/// Trait for providing an interface to a read-only NFT-like set of asset instances.
pub trait Inspect<AccountId> {
	/// Type for identifying an asset instance.
	type InstanceId;

	/// Type for identifying an asset class.
	type ClassId;

	/// Returns the owner of asset `instance` of `class`, or `None` if the asset doesn't exist (or
	/// somehow has no owner).
	fn owner(class: &Self::ClassId, instance: &Self::InstanceId) -> Option<AccountId>;

	/// Returns the asset instances of `class` owned by `who`.
	fn items(class: &Self::ClassId, who: &AccountId) -> Vec<Self::InstanceId>;

	/// Returns the attribute value of `instance` of `class` corresponding to `key`.
	///
	/// By default this is `None`; no attributes are defined.
	fn attribute(_class: &Self::ClassId, _instance: &Self::InstanceId, _key: &[u8])
		-> Option<Vec<u8>>
	{
		None
	}

	/// Returns the strongly-typed attribute value of `instance` of `class` corresponding to `key`.
	///
	/// By default this just attempts to use `attribute`.
	fn typed_attribute<K: Encode, V: Decode>(
		class: &Self::ClassId,
		instance: &Self::InstanceId,
		key: &K,
	) -> Option<V> {
		key.using_encoded(|d| Self::attribute(class, instance, d))
			.and_then(|v| V::decode(&mut &v[..]).ok())
	}

	/// Returns `true` if the asset `instance` of `class` may be transferred.
	///
	/// Default implementation is that all assets are transferable.
	fn can_transfer(_class: &Self::ClassId, _instance: &Self::InstanceId) -> bool { true }
}

/// Trait for providing an interface for multiple classes of NFT-like assets which may be minted,
/// burned and/or have attributes set on them.
pub trait Mutate<AccountId>: Inspect<AccountId> {
	/// Mint some asset `instance` of `class` to be owned by `who`.
	///
	/// By default, this is not a supported operation.
	fn mint_into(
		_class: &Self::ClassId,
		_instance: &Self::InstanceId,
		_who: &AccountId,
	) -> DispatchResult {
		Err(TokenError::Unsupported.into())
	}

	/// Burn some asset `instance` of `class`.
	///
	/// By default, this is not a supported operation.
	fn burn_from(_class: &Self::ClassId, _instance: &Self::InstanceId) -> DispatchResult {
		Err(TokenError::Unsupported.into())
	}

	/// Set attribute `value` of asset `instance` of `class`'s `key`.
	///
	/// By default, this is not a supported operation.
	fn set_attribute(
		_class: &Self::ClassId,
		_instance: &Self::InstanceId,
		_key: &[u8],
		_value: &[u8],
	) -> DispatchResult {
		Err(TokenError::Unsupported.into())
	}

	/// Attempt to set the strongly-typed attribute `value` of `instance` of `class`'s `key`.
	///
	/// By default this just attempts to use `set_attribute`.
	fn set_typed_attribute<K: Encode, V: Encode>(
		class: &Self::ClassId,
		instance: &Self::InstanceId,
		key: &K,
		value: &V,
	) -> DispatchResult {
		key.using_encoded(|k| value.using_encoded(|v|
			Self::set_attribute(class, instance, k, v)
		))
	}
}

/// Trait for providing a non-fungible sets of assets which can only be transferred.
pub trait Transfer<AccountId>: Inspect<AccountId> {
	/// Transfer asset `instance` of `class` into `destination` account.
	fn transfer(
		class: &Self::ClassId,
		instance: &Self::InstanceId,
		destination: &AccountId,
	) -> DispatchResult;
}
