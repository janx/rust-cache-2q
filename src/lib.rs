//! A 2Q cache
//!
//! This cache based on the paper entitled
//! **[2Q: A Low Overhead High-Performance Buffer Management Replacement Algorithm](http://www.vldb.org/conf/1994/P439.PDF)**.
#![deny(
    missing_docs,
    missing_debug_implementations, missing_copy_implementations,
    trivial_casts, trivial_numeric_casts,
    unsafe_code,
    unstable_features,
    unused_import_braces, unused_qualifications
 )]

use std::collections::VecDeque;
use std::collections::vec_deque;
use std::borrow::Borrow;
use std::cmp;
use std::mem;
use std::iter;
use std::fmt;

/// The type of items in the recent and frequent lists.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct CacheEntry<K, V> {
    key: K,
    value: V,
}

impl<'a, K, V> Into<(&'a K, &'a V)> for &'a CacheEntry<K, V> {
    fn into(self) -> (&'a K, &'a V) {
        (&self.key, &self.value)
    }
}

impl<'a, K, V> Into<(&'a K, &'a mut V)> for &'a mut CacheEntry<K, V> {
    fn into(self) -> (&'a K, &'a mut V) {
        (&self.key, &mut self.value)
    }
}

/// A 2Q Cache which maps keys to values
///
/// 2Q is an enhancement over an LRU cache by tracking both recent and frequently accessed entries
/// separately. This avoids the cache being trashed by a scan of many new items: Only the recent
/// list will be trashed.
///
/// The cache is split into 3 sections, recent entries, frequent entries, and ghost entries.
///
/// * recent contains the most recently added entries.
/// * frequent is an LRU cache which contains entries which are frequently accessed
/// * ghost contains the keys which have been recently evicted from the recent cache.
///
/// New entries in the cache are initially placed in recent.
/// After recent fills up, the oldest entry from recent will be removed, and its key is placed in
/// ghost. When an entry is requested and not found, but its key is found in the ghost list,
/// an entry is pushed to the front of frequent.
///
/// # Examples
///
/// ```
/// use cache_2q::Cache;
///
/// // type inference lets us omit an explicit type signature (which
/// // would be `Cache<&str, &str>` in this example).
/// let mut book_reviews = Cache::new(1024);
///
/// // review some books.
/// book_reviews.insert("Adventures of Huckleberry Finn",    "My favorite book.");
/// book_reviews.insert("Grimms' Fairy Tales",               "Masterpiece.");
/// book_reviews.insert("Pride and Prejudice",               "Very enjoyable.");
/// book_reviews.insert("The Adventures of Sherlock Holmes", "Eye lyked it alot.");
///
/// // check for a specific one.
/// if !book_reviews.contains_key("Les Misérables") {
///     println!("We've got {} reviews, but Les Misérables ain't one.",
///              book_reviews.len());
/// }
///
/// // oops, this review has a lot of spelling mistakes, let's delete it.
/// book_reviews.remove("The Adventures of Sherlock Holmes");
///
/// // look up the values associated with some keys.
/// let to_find = ["Pride and Prejudice", "Alice's Adventure in Wonderland"];
/// for book in &to_find {
///     match book_reviews.get(book) {
///         Some(review) => println!("{}: {}", book, review),
///         None => println!("{} is unreviewed.", book)
///     }
/// }
///
/// // iterate over everything.
/// for (book, review) in &book_reviews {
///     println!("{}: \"{}\"", book, review);
/// }
/// ```
///
/// Cache also implements an Entry API, which allows for more complex methods of getting,
/// setting, updating and removing keys and their values:
///
/// ```
/// use cache_2q::Cache;
///
/// // type inference lets us omit an explicit type signature (which
/// // would be `Cache<&str, u8>` in this example).
/// let mut player_stats = Cache::new(32);
///
/// fn random_stat_buff() -> u8 {
///     // could actually return some random value here - let's just return
///     // some fixed value for now
///     42
/// }
///
/// // insert a key only if it doesn't already exist
/// player_stats.entry("health").or_insert(100);
///
/// // insert a key using a function that provides a new value only if it
/// // doesn't already exist
/// player_stats.entry("defence").or_insert_with(random_stat_buff);
///
/// // update a key, guarding against the key possibly not being set
/// let stat = player_stats.entry("attack").or_insert(100);
/// *stat += random_stat_buff();
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cache<K, V> {
    frequent: VecDeque<CacheEntry<K, V>>,
    recent: VecDeque<CacheEntry<K, V>>,
    ghost: VecDeque<K>,
    max_frequent: usize,
    max_recent: usize,
    max_ghost: usize,
}

impl<K: Eq, V> Cache<K, V> {
    /// Creates an empty cache, with the specified size
    ///
    /// # Notes
    /// `size` defines the maximum number of entries, but there can be
    /// an additional `size / 2` instances of `K`
    ///
    /// # Examples
    ///
    /// ```
    /// use cache_2q::Cache;
    ///
    /// let mut cache: Cache<u64, Vec<u8>> = Cache::new(8);
    /// cache.insert(1, vec![1,2,3,4]);
    /// assert_eq!(*cache.get(&1).unwrap(), &[1,2,3,4]);
    /// ```
    ///
    /// # Panics
    /// panics if `size` is zero. A zero-sized cache isn't very useful, and breaks some apis
    /// (like [VacantEntry::insert], which returns a reference to the newly inserted item)
    ///
    /// [VacantEntry::insert]: struct.VacantEntry.html#method.insert
    pub fn new(size: usize) -> Cache<K, V> {
        assert!(size > 0);
        let max_recent = cmp::max(1, size / 4);
        let max_frequent = size - max_recent;
        let max_ghost = size / 2;
        Cache {
            frequent: VecDeque::with_capacity(max_frequent),
            recent: VecDeque::with_capacity(max_recent),
            ghost: VecDeque::with_capacity(max_ghost),
            max_frequent: max_frequent,
            max_recent: max_recent,
            max_ghost: max_ghost,
        }
    }

    /// Returns true if the cache contains a value for the specified key.
    ///
    /// The key may be any borrowed form of the cache's key type, but
    /// Eq on the borrowed form must match those for the key type.
    ///
    /// # Examples
    ///
    /// ```
    /// use cache_2q::Cache;
    ///
    /// let mut cache = Cache::new(8);
    /// cache.insert(1, "a");
    /// assert_eq!(cache.contains_key(&1), true);
    /// assert_eq!(cache.contains_key(&2), false);
    /// ```
    pub fn contains_key<Q: ?Sized>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Eq,
    {
        self.recent.iter().any(|entry| entry.key.borrow() == key) ||
            self.frequent.iter().any(|entry| entry.key.borrow() == key)
    }

    /// Returns a reference to the value corresponding to the key.
    ///
    /// The key may be any borrowed form of the cache's key type, but Eq on the borrowed form
    /// must match those for the key type.
    ///
    /// Unlike [get()], the the cache will not be updated to reflect a new access of `key`.
    /// Because the cache is not updated, `peek()` can operate without mutable access to the cache
    ///
    /// # Examples
    ///
    /// ```
    /// use cache_2q::Cache;
    ///
    /// let mut cache = Cache::new(32);
    /// cache.insert(1, "a");
    /// let cache = cache;
    /// // peek doesn't require mutable access to the cache
    /// assert_eq!(cache.peek(&1), Some(&"a"));
    /// assert_eq!(cache.peek(&2), None);
    /// ```
    ///
    /// [get()]: struct.Cache.html#method.get
    pub fn peek<Q: ?Sized>(&self, key: &Q) -> Option<&V>
        where
        K: Borrow<Q>,
        Q: Eq,
    {
        if let Some(&CacheEntry { ref value, .. }) =
            self.recent.iter().find(|entry| entry.key.borrow() == key)
        {
            Some(value)
        } else if let Some(&CacheEntry { ref value, .. }) =
            self.frequent.iter().find(|entry| entry.key.borrow() == key)
        {
            Some(value)
        } else {
            None
        }

    }

    /// Returns a reference to the value corresponding to the key.
    ///
    /// The key may be any borrowed form of the cache's key type, but Eq on the borrowed form
    /// must match those for the key type.
    ///
    /// # Examples
    ///
    /// ```
    /// use cache_2q::Cache;
    ///
    /// let mut cache = Cache::new(32);
    /// cache.insert(1, "a");
    /// assert_eq!(cache.get(&1), Some(&"a"));
    /// assert_eq!(cache.get(&2), None);
    /// ```
    pub fn get<Q: ?Sized>(&mut self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Eq,
    {
        if let Some(&CacheEntry { ref value, .. }) =
            self.recent.iter().find(|entry| entry.key.borrow() == key)
        {
            Some(value)
        } else if let Some(i) = self.frequent.iter().position(|entry| entry.key.borrow() == key) {
            let old = self.frequent.remove(i).unwrap();
            self.frequent.push_front(old);
            Some(&self.frequent[0].value)
        } else {
            None
        }
    }

    /// Returns a mutable reference to the value corresponding to the key.
    ///
    /// The key may be any borrowed form of the cache's key type, but
    /// Eq on the borrowed form *must* match those for
    /// the key type.
    ///
    /// # Examples
    ///
    /// ```
    /// use cache_2q::Cache;
    ///
    /// let mut cache = Cache::new(8);
    /// cache.insert(1, "a");
    /// if let Some(x) = cache.get_mut(&1) {
    ///     *x = "b";
    /// }
    /// assert_eq!(cache.get(&1), Some(&"b"));
    /// ```
    pub fn get_mut<Q: ?Sized>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
        Q: Eq,
    {
        if let Some(&mut CacheEntry { ref mut value, .. }) = self.recent
            .iter_mut()
            .find(|entry| entry.key.borrow() == key)
        {
            Some(value)
        } else if let Some(i) = self.frequent.iter().position(|entry| entry.key.borrow() == key) {
            let old = self.frequent.remove(i).unwrap();
            self.frequent.push_front(old);
            Some(&mut self.frequent[0].value)
        } else {
            None
        }
    }

    /// Inserts a key-value pair into the cache.
    ///
    /// If the cache did not have this key present, None is returned.
    ///
    /// If the cache did have this key present, the value is updated, and the old
    /// value is returned.
    ///
    /// # Examples
    ///
    /// ```
    /// use cache_2q::Cache;
    ///
    /// let mut cache = Cache::new(8);
    /// assert_eq!(cache.insert(37, "a"), None);
    /// assert_eq!(cache.is_empty(), false);
    ///
    /// cache.insert(37, "b");
    /// assert_eq!(cache.insert(37, "c"), Some("b"));
    /// assert_eq!(*cache.get(&37).unwrap(), "c");
    /// ```
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        match self.entry(key) {
            Entry::Occupied(mut entry) => Some(entry.insert(value)),
            Entry::Vacant(entry) => {
                entry.insert(value);
                None
            }
        }
    }

    /// Gets the given key's corresponding entry in the cache for in-place manipulation.
    ///
    /// # Examples
    ///
    /// ```
    /// use cache_2q::Cache;
    ///
    /// let mut stringified = Cache::new(8);
    ///
    /// for &i in &[1, 2, 5, 1, 2, 8, 1, 2, 102, 25, 1092, 1, 2, 82, 10, 1095] {
    ///     let string = stringified.entry(i).or_insert_with(|| i.to_string());
    ///     assert_eq!(string, &i.to_string());
    /// }
    /// ```
    pub fn entry(&mut self, key: K) -> Entry<K, V> {
        let mut entry = self.peek_entry(key);
        if let Entry::Occupied(OccupiedEntry {
            ref mut cache,
            kind: OccupiedKind::Frequent(ref mut i),
            ..
        }) = entry
        {
            let old_entry = cache.frequent.remove(*i).unwrap();
            cache.frequent.push_front(old_entry);
            *i = 0;
        }
        entry
    }

    /// Returns the number of entries currenly in the cache.
    ///
    /// # Examples
    ///
    /// ```
    /// use cache_2q::Cache;
    ///
    /// let mut a = Cache::new(8);
    /// assert_eq!(a.len(), 0);
    /// a.insert(1, "a");
    /// assert_eq!(a.len(), 1);
    /// ```
    pub fn len(&self) -> usize {
        self.recent.len() + self.frequent.len()
    }

    /// Returns true if the cache contains no elements.
    ///
    /// # Examples
    ///
    /// ```
    /// use cache_2q::Cache;
    ///
    /// let mut a = Cache::new(8);
    /// assert!(a.is_empty());
    /// a.insert(1, "a");
    /// assert!(!a.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.recent.is_empty() && self.frequent.is_empty()
    }

    /// Removes a key from the cache, returning the value associated with the key if the key
    /// was previously in the cache.
    ///
    /// The key may be any borrowed form of the cache's key type, but
    /// Eq on the borrowed form *must* match those for
    /// the key type.
    ///
    /// # Examples
    ///
    /// ```
    /// use cache_2q::Cache;
    ///
    /// let mut cache = Cache::new(8);
    /// cache.insert(1, "a");
    /// assert_eq!(cache.remove(&1), Some("a"));
    /// assert_eq!(cache.remove(&1), None);
    /// ```
    pub fn remove<Q: ?Sized>(&mut self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Eq,
    {
        if let Some(i) = self.recent
            .iter()
            .position(|entry| entry.key.borrow() == key)
        {
            Some(self.recent.remove(i).unwrap().value)
        } else if let Some(i) = self.frequent.iter().position(|entry| entry.key.borrow() == key) {
            Some(self.frequent.remove(i).unwrap().value)
        } else {
            None
        }
    }

    /// Clears the cache, removing all key-value pairs. Keeps the allocated memory for reuse.
    ///
    /// # Examples
    ///
    /// ```
    /// use cache_2q::Cache;
    ///
    /// let mut a = Cache::new(32);
    /// a.insert(1, "a");
    /// a.clear();
    /// assert!(a.is_empty());
    /// ```
    pub fn clear(&mut self) {
        self.recent.clear();
        self.ghost.clear();
        self.frequent.clear();
    }

    /// Gets the given key's corresponding entry in the cache for in-place manipulation.
    /// The LRU portion of the cache is not updated
    ///
    /// # Examples
    ///
    /// ```
    /// use cache_2q::Cache;
    ///
    /// let mut stringified = Cache::new(8);
    ///
    /// for &i in &[1, 2, 5, 1, 2, 8, 1, 2, 102, 25, 1092, 1, 2, 82, 10, 1095] {
    ///     let string = stringified.peek_entry(i).or_insert_with(|| i.to_string());
    ///     assert_eq!(string, &i.to_string());
    /// }
    /// ```
    pub fn peek_entry(&mut self, key: K) -> Entry<K, V> {
        if let Some(i) = self.frequent.iter().position(|entry| &entry.key == &key) {
            Entry::Occupied(OccupiedEntry {
                cache: self,
                kind: OccupiedKind::Frequent(i),
            })
        } else if let Some(i) = self.recent.iter().position(|entry| &entry.key == &key) {
            Entry::Occupied(OccupiedEntry {
                cache: self,
                kind: OccupiedKind::Recent(i),
            })
        } else if let Some(i) = self.ghost.iter().position(|old_key| old_key == &key) {
            Entry::Vacant(VacantEntry {
                cache: self,
                key: key,
                kind: VacantKind::Ghost(i),
            })
        } else {
            Entry::Vacant(VacantEntry {
                cache: self,
                key: key,
                kind: VacantKind::Unknown,
            })
        }
    }

    /// An iterator visiting all key-value pairs in arbitrary order.
    /// The iterator element type is `(&'a K, &'a V)`.
    ///
    /// # Examples
    ///
    /// ```
    /// use cache_2q::Cache;
    ///
    /// let mut cache = Cache::new(8);
    /// cache.insert("a", 1);
    /// cache.insert("b", 2);
    /// cache.insert("c", 3);
    ///
    /// for (key, val) in cache.iter() {
    ///     println!("key: {} val: {}", key, val);
    /// }
    /// ```
    pub fn iter(&self) -> Iter<K, V> {
        Iter {
            inner: self.recent.iter().chain(self.frequent.iter()).map(Into::into),
        }
    }
}

impl<'a, K: 'a + Eq, V: 'a> IntoIterator for &'a Cache<K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter = Iter<'a, K, V>;
    fn into_iter(self) -> Iter<'a, K, V> {
        self.iter()
    }
}

/// A view into a single entry in a cache, which may either be vacant or occupied.
///
/// This enum is constructed from the entry method on Cache.
pub enum Entry<'a, K: 'a, V: 'a> {
    /// An occupied entry
    Occupied(OccupiedEntry<'a, K, V>),
    /// An vacant entry
    Vacant(VacantEntry<'a, K, V>),
}

impl<'a, K: 'a + fmt::Debug, V: 'a + fmt::Debug> fmt::Debug for Entry<'a, K, V> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Entry::Vacant(ref v) => f.debug_tuple("Entry").field(v).finish(),
            Entry::Occupied(ref o) => f.debug_tuple("Entry").field(o).finish(),
        }
    }
}

impl<'a, K: 'a + Eq, V: 'a> Entry<'a, K, V> {
    /// Returns a reference to this entry's key.
    ///
    /// # Examples
    ///
    /// ```
    /// use cache_2q::Cache;
    ///
    /// let mut cache: Cache<&str, u32> = Cache::new(8);
    /// assert_eq!(cache.entry("poneyland").key(), &"poneyland");
    /// ```
    pub fn key(&self) -> &K {
        match *self {
            Entry::Occupied(ref entry) => entry.key(),
            Entry::Vacant(ref entry) => entry.key(),
        }
    }

    /// Ensures a value is in the entry by inserting the default if empty, and returns a mutable
    /// reference to the value in the entry.
    ///
    /// # Examples
    /// ```
    /// use cache_2q::Cache;
    ///
    /// let mut cache = Cache::new(8);
    /// {
    ///     let value = cache.entry(0xFF00).or_insert(0);
    ///     assert_eq!(*value, 0);
    /// }
    ///
    /// *cache.entry(0xFF00).or_insert(100) += 1;
    /// assert_eq!(*cache.get(&0xFF00).unwrap(), 1);
    /// ```
    pub fn or_insert(self, default: V) -> &'a mut V {
        match self {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(default),
        }
    }

    /// Ensures a value is in the entry by inserting the result of the default function if empty,
    /// and returns a mutable reference to the value in the entry.
    ///
    /// # Examples
    ///
    /// ```
    /// use cache_2q::Cache;
    ///
    /// let mut cache: Cache<&'static str, String> = Cache::new(8);
    /// cache.entry("key").or_insert_with(|| "value".to_string());
    ///
    /// assert_eq!(cache.get(&"key").unwrap(), &"value".to_string());
    /// ```
    pub fn or_insert_with<F: FnOnce() -> V>(self, default: F) -> &'a mut V {
        match self {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(default()),
        }
    }
}

/// A view into an occupied entry in a [`Cache`].
/// It is part of the [`Entry`] enum.
///
/// [`Cache`]: struct.Cache.html
/// [`Entry`]: enum.Entry.html
pub struct OccupiedEntry<'a, K: 'a, V: 'a> {
    cache: &'a mut Cache<K, V>,
    kind: OccupiedKind,
}

impl<'a, K: 'a + fmt::Debug, V: 'a + fmt::Debug> fmt::Debug for OccupiedEntry<'a, K, V> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("OccupiedEntry")
            .field("key", self.key())
            .field("value", self.get())
            .field(
                "kind",
                &if let OccupiedKind::Frequent(_) = self.kind {
                    "frequent"
                } else {
                    "recent"
                },
            )
            .finish()
    }
}

impl<'a, K: 'a, V: 'a> OccupiedEntry<'a, K, V> {
    fn entry(&self) -> &CacheEntry<K, V> {
        match self.kind {
            OccupiedKind::Recent(idx) => &self.cache.recent[idx],
            OccupiedKind::Frequent(idx) => &self.cache.frequent[idx],
        }
    }
    fn entry_mut(&mut self) -> &mut CacheEntry<K, V> {
        match self.kind {
            OccupiedKind::Recent(idx) => &mut self.cache.recent[idx],
            OccupiedKind::Frequent(idx) => &mut self.cache.frequent[idx],
        }
    }

    /// Gets a reference to the key in the entry.
    ///
    /// # Examples
    ///
    /// ```
    /// use cache_2q::{Cache, Entry};
    ///
    /// let mut cache: Cache<&str, u32> = Cache::new(8);
    /// cache.entry("poneyland").or_insert(12);
    /// match cache.entry("poneyland") {
    ///     Entry::Vacant(_) => {
    ///         panic!("Should be occupied");
    ///     },
    ///     Entry::Occupied(occupied) => {
    ///         assert_eq!(occupied.key(), &"poneyland");
    ///     },
    /// }
    /// ```
    pub fn key(&self) -> &K {
        &self.entry().key
    }

    /// Gets a reference to the value in the entry.
    ///
    /// # Examples
    ///
    /// ```
    /// use cache_2q::{Cache, Entry};
    ///
    /// let mut cache: Cache<&str, u32> = Cache::new(8);
    /// cache.entry("poneyland").or_insert(12);
    ///
    /// if let Entry::Occupied(o) = cache.entry("poneyland") {
    ///     assert_eq!(o.get(), &12);
    /// } else {
    ///     panic!("Entry should be occupied");
    /// }
    /// ```
    pub fn get(&self) -> &V {
        &self.entry().value
    }

    /// Gets a mutable reference to the value in the entry.
    ///
    /// # Examples
    ///
    /// ```
    /// use cache_2q::{Cache, Entry};
    ///
    /// let mut cache: Cache<&str, u32> = Cache::new(8);
    /// cache.entry("poneyland").or_insert(12);
    ///
    /// assert_eq!(*cache.get("poneyland").unwrap(), 12);
    /// if let Entry::Occupied(mut o) = cache.entry("poneyland") {
    ///      *o.get_mut() += 10;
    /// } else {
    ///     panic!("Entry should be occupied");
    /// }
    ///
    /// assert_eq!(*cache.get("poneyland").unwrap(), 22);
    /// ```
    pub fn get_mut(&mut self) -> &mut V {
        &mut self.entry_mut().value
    }

    /// Converts the OccupiedEntry into a mutable reference to the value in the entry
    /// with a lifetime bound to the cache itself.
    ///
    /// # Examples
    ///
    /// ```
    /// use cache_2q::{Cache, Entry};
    ///
    /// let mut cache: Cache<&str, u32> = Cache::new(8);
    /// cache.entry("poneyland").or_insert(12);
    ///
    /// assert_eq!(*cache.get("poneyland").unwrap(), 12);
    /// if let Entry::Occupied(o) = cache.entry("poneyland") {
    ///     *o.into_mut() += 10;
    /// } else {
    ///     panic!("Entry should be occupied");
    /// }
    ///
    /// assert_eq!(*cache.get("poneyland").unwrap(), 22);
    /// ```
    pub fn into_mut(self) -> &'a mut V {
        match self.kind {
            OccupiedKind::Recent(idx) => &mut self.cache.recent[idx].value,
            OccupiedKind::Frequent(idx) => &mut self.cache.frequent[idx].value,
        }
    }

    /// Sets the value of the entry, and returns the entry's old value.
    ///
    /// # Examples
    ///
    /// ```
    /// use cache_2q::{Cache, Entry};
    ///
    /// let mut cache: Cache<&str, u32> = Cache::new(8);
    /// cache.entry("poneyland").or_insert(12);
    ///
    /// if let Entry::Occupied(mut o) = cache.entry("poneyland") {
    ///     assert_eq!(o.insert(15), 12);
    /// } else {
    ///     panic!("Entry should be occupied");
    /// }
    ///
    /// assert_eq!(*cache.get("poneyland").unwrap(), 15);
    /// ```
    pub fn insert(&mut self, value: V) -> V {
        mem::replace(self.get_mut(), value)
    }

    /// Take the ownership of the key and value from the cache.
    ///
    /// # Examples
    ///
    /// ```
    /// use cache_2q::{Cache, Entry};
    ///
    /// let mut cache: Cache<&str, u32> = Cache::new(8);
    /// cache.entry("poneyland").or_insert(12);
    ///
    /// if let Entry::Occupied(o) = cache.entry("poneyland") {
    ///     // We delete the entry from the cache.
    ///     o.remove_entry();
    /// } else {
    ///     panic!("Entry should be occupied");
    /// }
    ///
    /// assert_eq!(cache.contains_key("poneyland"), false);
    /// ```
    pub fn remove_entry(self) -> (K, V) {
        match self.kind {
            OccupiedKind::Recent(idx) => {
                let entry = self.cache.recent.remove(idx).unwrap();
                (entry.key, entry.value)
            }
            OccupiedKind::Frequent(idx) => {
                let entry = self.cache.frequent.remove(idx).unwrap();
                (entry.key, entry.value)
            }
        }
    }

    /// Takes the value out of the entry, and returns it.
    ///
    /// # Examples
    ///
    /// ```
    /// use cache_2q::{Cache, Entry};
    ///
    /// let mut cache: Cache<&str, u32> = Cache::new(8);
    /// cache.entry("poneyland").or_insert(12);
    ///
    /// if let Entry::Occupied(o) = cache.entry("poneyland") {
    ///     assert_eq!(o.remove(), 12);
    /// } else {
    ///     panic!("Entry should be occupied");
    /// }
    ///
    /// assert_eq!(cache.contains_key("poneyland"), false);
    /// ```
    pub fn remove(self) -> V {
        self.remove_entry().1
    }
}

/// A view into a vacant entry in a [`Cache`].
/// It is part of the [`Entry`] enum.
///
/// [`Cache`]: struct.Cache.html
/// [`Entry`]: enum.Entry.html
pub struct VacantEntry<'a, K: 'a, V: 'a> {
    cache: &'a mut Cache<K, V>,
    key: K,
    kind: VacantKind,
}

impl<'a, K: 'a + fmt::Debug, V: 'a + fmt::Debug> fmt::Debug for VacantEntry<'a, K, V> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("VacantEntry")
            .field("key", self.key())
            .field(
                "remembered",
                &if let VacantKind::Ghost(_) = self.kind {
                    true
                } else {
                    false
                },
            )
            .finish()
    }
}

impl<'a, K: 'a, V: 'a> VacantEntry<'a, K, V> {
    /// Gets a reference to the key that would be used when inserting a value
    /// through the `VacantEntry`.
    ///
    /// # Examples
    ///
    /// ```
    /// use cache_2q::{Cache, Entry};
    ///
    /// let mut cache: Cache<&str, u32> = Cache::new(8);
    ///
    /// if let Entry::Vacant(v) = cache.entry("poneyland") {
    ///     assert_eq!(v.key(), &"poneyland");
    /// } else {
    ///     panic!("Entry should be vacant");
    /// }
    /// ```
    pub fn key(&self) -> &K {
        &self.key
    }

    /// Take ownership of the key.
    ///
    /// # Examples
    ///
    /// ```
    /// use cache_2q::{Cache, Entry};
    ///
    /// let mut cache: Cache<String, u32> = Cache::new(8);
    ///
    /// if let Entry::Vacant(v) = cache.entry("poneyland".into()) {
    ///     assert_eq!(v.into_key(), "poneyland".to_string());
    /// } else {
    ///     panic!("Entry should be vacant");
    /// }
    /// ```
    pub fn into_key(self) -> K {
        self.key
    }
}

impl<'a, K: 'a + Eq, V: 'a> VacantEntry<'a, K, V> {
    /// Sets the value of the entry with the VacantEntry's key,
    /// and returns a mutable reference to it.
    ///
    /// # Examples
    ///
    /// ```
    /// use cache_2q::{Cache, Entry};
    ///
    /// let mut cache: Cache<&str, u32> = Cache::new(8);
    ///
    /// if let Entry::Vacant(o) = cache.entry("poneyland") {
    ///     o.insert(37);
    /// } else {
    ///     panic!("Entry should be vacant");
    /// }
    /// assert_eq!(*cache.get("poneyland").unwrap(), 37);
    /// ```
    pub fn insert(self, value: V) -> &'a mut V {
        let VacantEntry { cache, key, kind } = self;
        match kind {
            VacantKind::Ghost(idx) => {
                cache.ghost.remove(idx);
                if cache.frequent.len() + 1 > cache.max_frequent {
                    cache.frequent.pop_back();
                }
                cache.frequent.push_front(CacheEntry {
                    key: key,
                    value: value,
                });
                &mut cache.frequent[0].value
            }
            VacantKind::Unknown => {
                if cache.recent.len() + 1 > cache.max_recent {
                    if let Some(CacheEntry {key: old_key, ..}) = cache.recent.pop_back() {
                        if cache.ghost.len() + 1 > cache.max_ghost {
                            cache.ghost.pop_back();
                        }
                        cache.ghost.push_front(old_key);
                    }
                }
                cache.recent.push_front(CacheEntry {
                    key: key,
                    value: value,
                });
                &mut cache.recent[0].value
            }
        }
    }
}

/// An iterator over the entries of a `Cache`.
///
/// This `struct` is created by the [`iter`] method on [`Cache`]. See its
/// documentation for more.
///
/// [`iter`]: struct.Cache.html#method.iter
/// [`Cache`]: struct.Cache.html
pub struct Iter<'a, K: 'a, V: 'a> {
    inner: iter::Map<
        iter::Chain<vec_deque::Iter<'a, CacheEntry<K, V>>, vec_deque::Iter<'a, CacheEntry<K, V>>>,
        fn(&'a CacheEntry<K, V>) -> (&K, &V),
    >,
}

impl<'a, K: 'a, V: 'a> Clone for Iter<'a, K, V> {
    fn clone(&self) -> Self {
        Iter {
            inner: self.inner.clone(),
        }
    }
}

impl<'a, K: 'a + fmt::Debug, V: 'a + fmt::Debug> fmt::Debug for Iter<'a, K, V> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_list().entries(self.clone()).finish()
    }
}

impl<'a, K: 'a, V: 'a> Iterator for Iter<'a, K, V> {
    type Item = (&'a K, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum VacantKind {
    Ghost(usize),
    Unknown,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum OccupiedKind {
    Recent(usize),
    Frequent(usize),
}

#[cfg(test)]
mod tests {
    use super::Cache;

    #[test]
    fn cache_zero_size() {
        let mut cache = Cache::new(8);
        for _ in 0..1024 {
            cache.entry(()).or_insert_with(|| ());
        }
    }

    #[test]
    fn get_borrowed() {
        let mut cache = Cache::new(8);
        cache.entry("hi".to_string()).or_insert(0);
        cache.entry("there".to_string()).or_insert(0);
        assert_eq!(*cache.get("hi").unwrap(), 0);
    }

    #[test]
    #[should_panic]
    fn empty_cache() {
        Cache::<(), ()>::new(0);
    }

    #[test]
    fn size_1_cache() {
        let mut cache = Cache::new(1);
        cache.insert(100, "value");
        assert_eq!(cache.get(&100), Some(&"value"));
        cache.insert(200, "other");
        assert_eq!(cache.get(&200), Some(&"other"));
        assert_eq!(cache.get(&100), None);
    }
}
