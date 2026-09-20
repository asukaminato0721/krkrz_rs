//! Enumeration order of tTJSCustomObject (tjsObject.cpp / tjsHashSearch.h).
//! Values remain in the VM's map; this tracks occupied bucket heads and chains.
#[derive(Clone, Debug, Default)]
struct Bucket {
    head: Option<Vec<u16>>,
    chain: Vec<Vec<u16>>,
}

#[derive(Clone, Debug)]
pub(crate) struct MemberLayout {
    buckets: Vec<Bucket>,
}

impl Default for MemberLayout {
    fn default() -> Self {
        Self {
            buckets: vec![Bucket::default(); 8],
        }
    }
}

fn hash(key: &[u16]) -> u32 {
    let mut hash = 0u32;
    for unit in key.iter().take_while(|unit| **unit != 0) {
        hash = hash.wrapping_add(u32::from(*unit));
        hash = hash.wrapping_add(hash << 10);
        hash ^= hash >> 6;
    }
    hash = hash.wrapping_add(hash << 3);
    hash ^= hash >> 11;
    hash = hash.wrapping_add(hash << 15);
    if hash == 0 { u32::MAX } else { hash }
}

impl MemberLayout {
    pub fn rehash(&mut self, count: usize) {
        let bits = if count == 0 { 2 } else { count.ilog2() + 2 };
        let size = 1usize << bits;
        if self.buckets.len() == size {
            return;
        }
        let keys = self.keys();
        self.buckets = vec![Bucket::default(); size];
        for key in keys {
            self.insert(&key);
        }
    }
    fn bucket(&mut self, key: &[u16]) -> &mut Bucket {
        let index = hash(key) as usize & (self.buckets.len() - 1);
        &mut self.buckets[index]
    }
    pub fn touch(&mut self, key: &[u16]) {
        let bucket = self.bucket(key);
        if let Some(index) = bucket.chain.iter().position(|item| item == key)
            && index > 2
        {
            let key = bucket.chain.remove(index);
            bucket.chain.insert(0, key);
        }
    }
    pub fn insert(&mut self, key: &[u16]) {
        self.touch(key);
        let bucket = self.bucket(key);
        if bucket.head.as_deref() == Some(key) || bucket.chain.iter().any(|item| item == key) {
            return;
        }
        if bucket.head.is_none() {
            bucket.head = Some(key.to_vec());
        } else {
            bucket.chain.insert(0, key.to_vec());
        }
    }
    pub fn remove(&mut self, key: &[u16]) {
        let bucket = self.bucket(key);
        if bucket.head.as_deref() == Some(key) {
            bucket.head = None;
        } else {
            bucket.chain.retain(|item| item != key);
        }
    }
    pub fn keys(&self) -> Vec<Vec<u16>> {
        self.buckets
            .iter()
            .flat_map(|bucket| bucket.chain.iter().chain(bucket.head.iter()))
            .cloned()
            .collect()
    }
}
