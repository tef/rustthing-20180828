use std::borrow::Borrow;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

type Address = usize;

pub struct FakeAtomicVec<T> {
    arr: Arc<Mutex<Vec<T>>>,
}

impl <T> FakeAtomicVec<T> {
    pub fn with_capacity(capacity: usize) -> FakeAtomicVec<T> {
        FakeAtomicVec {
            arr: Arc::new(Mutex::new(Vec::with_capacity(capacity))),
        }
    }
}



struct Delete<P> {
    epoch: usize,
    value: P,
}

pub struct Collector<'a, P: 'a> {
    table: &'a SharedHeap<P>,
    delete: FakeAtomicVec<Delete<P>>, // ArcMutex
    // read_set
} 

impl <'a, P> Collector<'a, P> {
    fn collect_later(&self, epoch: usize, value: *mut P) {
        unimplemented!()
    }
    pub fn collect(&mut self) {
        // read current epoch, 
    }
}

impl<'a, T> Drop for Collector<'a, T> {
    fn drop(&mut self) {
        // spin and collect ?
        ;
    }
}
pub struct SharedHeap<P> {
    arr: FakeAtomicVec<P>, // ArcMutex
    epoch: AtomicUsize
    // either cells or
}

impl <P> SharedHeap<P> {
    pub fn new(capacity: usize) -> SharedHeap<P> {
        let v = FakeAtomicVec::with_capacity(capacity);
        let e = AtomicUsize::new(0);
        SharedHeap {
            arr: v,
            epoch: e,
        }
    }

    fn current_epoch(&self) -> usize {
        self.epoch.load(Ordering::SeqCst)
    }

    pub fn collector<'a>(&'a self) -> Collector<'a, P> {
        let v = FakeAtomicVec::with_capacity(20);
        Collector {table:self, delete: v}
    }

    pub fn transaction<'a>(&'a self, collector: &'a Collector<'a, P>) -> Transaction<'a, P> {
        Transaction {table:self, collector: collector, epoch: self.current_epoch()} 
    }

}

pub struct Transaction<'a, P: 'a> {
    table: &'a SharedHeap<P>,
    collector: &'a Collector<'a, P>,
    epoch: usize,
    // read_set
}


impl <'a, P> Transaction<'a, P> {
    pub fn insert(&mut self, value: P) -> Result<Address, P> {
        // find first empty slot
        // turn box into a pointer
        // swap it inside.
        Ok(1)
    }
    
    pub fn upsert<U: Fn(&mut P)>(&mut self, address: Address, value: P, on_conflict: U) {
        unimplemented!()
    }

    pub fn delete(&mut self, address: Address) -> bool {
        unimplemented!()
    }

    pub fn borrow(&mut self, address:Address) -> &P {
        // in pessimistic, cas the address with a placeholder
        // add it to a list of things to return when done

        // in optimistic, don't cas it but
        // add it to a list of things to check when done
        
        unimplemented!()
    }
    pub fn copy(&mut self, address:Address) -> P {
        // same as read, but copying instead of borrowing
        unimplemented!()
    }
    pub fn replace(&mut self, address:Address, value:P) -> Result<P,P> {
        // same as read, placeholder
        // list of things to write when done

        // optimisitic has write list & if read set promotes it
        unimplemented!()
    }
    pub fn apply(&mut self) -> bool {
        // in pessimistic, unlock each item and repl        ace it

        // in optimistic, lock each item, validate unchanged
        // return
        unimplemented!()
    }
    pub fn cancel(&mut self) -> bool {
        // race to undo
        unimplemented!()
    }
}

impl<'a, T> Drop for Transaction<'a, T> {
    fn drop(&mut self) {
        // if !apply, cancel
        ;
    }
}


#[cfg(test)]
mod tests {
    use SharedHeap;
    use std::sync::Arc;
    #[test]
    fn it_works() {
        let h = Arc::new(SharedHeap::new(1024));
        let mut c = h.collector();
        let mut addr = 0;
        {
            let mut txn = h.transaction(&c);
            addr = txn.insert("example".to_string()).unwrap();
        }
        {
            let mut txn = h.transaction(&c);
            addr = txn.insert("example".to_string()).unwrap();
        }
        c.collect();
    }
}
