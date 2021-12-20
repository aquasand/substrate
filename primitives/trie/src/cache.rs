// This file is part of Substrate.

// Copyright (C) 2021 Parity Technologies (UK) Ltd.
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

use crate::Layout;
use hash_db::Hasher;
use parking_lot::{
	MappedRwLockWriteGuard, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard,
};
use std::{
	collections::{hash_map::Entry, HashMap},
	sync::Arc,
};
use trie_db::{node::NodeOwned, Bytes, CError};

pub struct SharedTrieNodeCache<H: Hasher> {
	node_cache: Arc<RwLock<HashMap<H::Out, NodeOwned<H::Out>>>>,
	data_cache: Option<Arc<RwLock<HashMap<H::Out, HashMap<Vec<u8>, Option<Bytes>>>>>>,
}

impl<H: Hasher> Clone for SharedTrieNodeCache<H> {
	fn clone(&self) -> Self {
		Self { node_cache: self.node_cache.clone(), data_cache: self.data_cache.clone() }
	}
}

impl<H: Hasher> SharedTrieNodeCache<H> {
	/// Create a new [`SharedTrieNodeCache`].
	///
	/// If `enable_data_cache` is `true`, the special data cache will be enabled. The data cache
	/// caches `key => data` per storage root. So, when trying to access some data in the trie using
	/// a key, we can directly look up the data instead of traversing the trie.
	pub fn new(enable_data_cache: bool) -> Self {
		Self {
			node_cache: Default::default(),
			data_cache: enable_data_cache.then(|| Default::default()),
		}
	}

	/// Create a new [`LocalTrieNodeCache`] instance from this shared cache.
	pub fn local_cache(&self) -> LocalTrieNodeCache<H> {
		LocalTrieNodeCache { shared: self.clone(), local: Default::default() }
	}
}

pub struct LocalTrieNodeCache<H: Hasher> {
	shared: SharedTrieNodeCache<H>,
	local: Mutex<HashMap<H::Out, NodeOwned<H::Out>>>,
}

impl<H: Hasher> LocalTrieNodeCache<H> {
	/// Return self as a [`TrieDB`](trie_db::TrieDB) compatible cache.
	///
	/// The given `storage_root` needs to be the storage root of the trie this cache is used for.
	pub fn as_trie_db_cache<'a>(&'a self, storage_root: H::Out) -> TrieNodeCache<'a, H> {
		let data_cache = if let Some(ref cache) = self.shared.data_cache {
			DataCache::ForStorageRoot(RwLockWriteGuard::map(cache.write(), |cache| {
				cache.entry(storage_root).or_default()
			}))
		} else {
			DataCache::Disabled
		};

		TrieNodeCache {
			shared_cache: self.shared.node_cache.read(),
			local_cache: self.local.lock(),
			data_cache,
		}
	}

	/// Return self as [`TrieDBMut`](trie_db::TrieDBMut) compatible cache.
	///
	/// After finishing all operations with [`TrieDBMut`](trie_db::TrieDBMut) and having obtained
	/// the new storage root, [`TrieNodeCache::merge_into`] should be called to update this local
	/// cache instance. If the function is not called, cached data is just thrown away and not
	/// propagated to the shared cache. So, accessing these new items will be slower, but nothing
	/// would break because of this.
	pub fn as_trie_db_mut_cache<'a>(&'a self) -> TrieNodeCache<'a, H> {
		TrieNodeCache {
			shared_cache: self.shared.node_cache.read(),
			local_cache: self.local.lock(),
			data_cache: DataCache::Fresh(Default::default()),
		}
	}
}

impl<H: Hasher> Drop for LocalTrieNodeCache<H> {
	fn drop(&mut self) {
		let mut shared = self.shared.node_cache.write();
		shared.extend(self.local.lock().drain());
	}
}

/// The abstraction of the data cache for the [`TrieNodeCache`].
enum DataCache<'a> {
	/// The data cache is disabled.
	Disabled,
	/// The data cache is fresh, aka not yet associated to any storage root.
	/// This is used for example when a new trie is being build, to cache new data.
	Fresh(HashMap<Vec<u8>, Option<Bytes>>),
	/// The data cache is already bound to a specific storage root.
	///
	/// The actual storage root is not stored here.
	ForStorageRoot(MappedRwLockWriteGuard<'a, HashMap<Vec<u8>, Option<Bytes>>>),
}

impl DataCache<'_> {
	/// Get the data for the given `key`.
	fn get(&self, key: &[u8]) -> Option<&Option<Bytes>> {
		match self {
			Self::Disabled => None,
			Self::Fresh(map) => map.get(key),
			Self::ForStorageRoot(map) => map.get(key),
		}
	}

	/// Insert some new `data` under the given `key`.
	fn insert(&mut self, key: &[u8], data: Option<Bytes>) {
		match self {
			Self::Disabled => {},
			Self::Fresh(map) => {
				map.insert(key.into(), data);
			},
			Self::ForStorageRoot(map) => {
				map.insert(key.into(), data);
			},
		}
	}
}

pub struct TrieNodeCache<'a, H: Hasher> {
	shared_cache: RwLockReadGuard<'a, HashMap<H::Out, NodeOwned<H::Out>>>,
	local_cache: MutexGuard<'a, HashMap<H::Out, NodeOwned<H::Out>>>,
	data_cache: DataCache<'a>,
}

impl<'a, H: Hasher> TrieNodeCache<'a, H> {
	/// Merge this cache into the given [`LocalTrieNodeCache`].
	///
	/// This function is only required to be called when this instance was created through
	/// [`LocalTrieNodeCache::as_trie_db_mut_cache`], otherwise this method is a no-op. The given
	/// `storage_root` is the new storage root that was obtained after finishing all operations
	/// using the [`TrieDBMut`](trie_db::TrieDBMut).
	pub fn merge_into(self, local: &LocalTrieNodeCache<H>, storage_root: H::Out) {
		let cache = if let DataCache::Fresh(cache) = self.data_cache { cache } else { return };

		if let Some(ref data_cache) = local.shared.data_cache {
			data_cache.write().entry(storage_root).or_default().extend(dbg!(cache));
		}
	}
}

impl<'a, H: Hasher> trie_db::TrieCache<Layout<H>> for TrieNodeCache<'a, H> {
	fn get_or_insert_node(
		&mut self,
		hash: H::Out,
		fetch_node: &mut dyn FnMut()
			-> trie_db::Result<NodeOwned<H::Out>, H::Out, CError<Layout<H>>>,
	) -> trie_db::Result<&NodeOwned<H::Out>, H::Out, CError<Layout<H>>> {
		if let Some(res) = self.shared_cache.get(&hash) {
			return Ok(res)
		}

		match self.local_cache.entry(hash) {
			Entry::Occupied(res) => Ok(res.into_mut()),
			Entry::Vacant(vacant) => {
				let node = (*fetch_node)()?;
				Ok(vacant.insert(node))
			},
		}
	}

	fn insert_node(&mut self, hash: H::Out, node: NodeOwned<H::Out>) {
		eprintln!("INSERT");
		self.local_cache.insert(hash, node);
	}

	fn get_node(&mut self, hash: &H::Out) -> Option<&NodeOwned<H::Out>> {
		if let Some(node) = self.shared_cache.get(hash) {
			return Some(node)
		}

		self.local_cache.get(hash)
	}

	fn lookup_data_for_key(&self, key: &[u8]) -> Option<&Option<Bytes>> {
		self.data_cache.get(key)
	}

	fn cache_data_for_key(&mut self, key: &[u8], data: Option<Bytes>) {
		eprintln!("INSERT2");
		self.data_cache.insert(key.into(), data);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use trie_db::{Trie, TrieDBBuilder, TrieDBMutBuilder, TrieHash, TrieMut};

	type MemoryDB = crate::MemoryDB<sp_core::Blake2Hasher>;
	type Layout = crate::Layout<sp_core::Blake2Hasher>;
	type Cache = super::SharedTrieNodeCache<sp_core::Blake2Hasher>;

	const TEST_DATA: &[(&[u8], &[u8])] =
		&[(b"key1", b"val1"), (b"key2", b"val2"), (b"key3", b"val3"), (b"key4", b"val4")];

	fn create_trie() -> (MemoryDB, TrieHash<Layout>) {
		let mut db = MemoryDB::default();
		let mut root = Default::default();

		{
			let mut trie = TrieDBMutBuilder::<Layout>::new(&mut db, &mut root).build();
			for (k, v) in TEST_DATA {
				trie.insert(k, v).expect("Inserts data");
			}
		}

		(db, root)
	}

	#[test]
	fn basic_cache_works() {
		let (db, root) = create_trie();

		let shared_cache = Cache::new(true);
		let local_cache = shared_cache.local_cache();

		{
			let mut cache = local_cache.as_trie_db_cache(root);
			let trie = TrieDBBuilder::<Layout>::new_unchecked(&db, &root)
				.with_cache(&mut cache)
				.build();
			assert_eq!(TEST_DATA[0].1.to_vec(), trie.get(TEST_DATA[0].0).unwrap().unwrap());
		}

		let cached_data = shared_cache
			.data_cache
			.as_ref()
			.unwrap()
			.read()
			.get(&root)
			.expect("There should be data cached")
			.get(TEST_DATA[0].0)
			.unwrap()
			.clone();
		assert_eq!(Bytes::from(TEST_DATA[0].1.to_vec()), cached_data.unwrap());
		// Local cache wasn't dropped yet, so there should not be any node cached.
		assert!(shared_cache.node_cache.read().is_empty());

		drop(local_cache);
		// Now we should have a value cached.
		assert!(shared_cache.node_cache.read().len() >= 1);

		let local_cache = shared_cache.local_cache();
		shared_cache
			.data_cache
			.as_ref()
			.unwrap()
			.write()
			.entry(root)
			.or_default()
			.insert(TEST_DATA[1].0.to_vec(), Some(b"fake_data".to_vec().into()));

		{
			let mut cache = local_cache.as_trie_db_cache(root);
			let trie = TrieDBBuilder::<Layout>::new_unchecked(&db, &root)
				.with_cache(&mut cache)
				.build();

			// We should now get the "fake_data", because we inserted this manually to the cache.
			assert_eq!(b"fake_data".to_vec(), trie.get(TEST_DATA[1].0).unwrap().unwrap());
		}
	}

	#[test]
	fn trie_db_mut_cache_works() {
		let (mut db, root) = create_trie();

		let new_key = b"new_key".to_vec();
		// Use some long value to not have it inlined
		let new_value = vec![23; 64];

		let shared_cache = Cache::new(true);
		let local_cache = shared_cache.local_cache();

		let mut new_root = root;
		let mut cache = local_cache.as_trie_db_mut_cache();

		{
			let mut trie = TrieDBMutBuilder::<Layout>::from_existing(&mut db, &mut new_root).unwrap()
				.with_cache(&mut cache)
				.build();

			trie.insert(&new_key, &new_value).unwrap();
		}

		cache.merge_into(&local_cache, new_root);

		let cached_data = shared_cache
			.data_cache
			.as_ref()
			.unwrap()
			.read()
			.get(&new_root)
			.expect("There should be data cached")
			.get(&new_key)
			.unwrap()
			.clone();
		assert_eq!(Bytes::from(new_value), cached_data.unwrap());
	}
}
