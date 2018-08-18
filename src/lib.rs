use std::borrow::Borrow;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

type Address = usize;

pub struct AtomicBox<T> {
    cell: Mutex<Box<T>>,
}

impl <T> AtomicBox<T> {
    pub fn new(value: T) -> AtomicBox<T> {
        AtomicBox { cell: Mutex::new(Box::new(value)) }
    }
    pub fn read(&self) -> *const T {
        unimplemented!()
    }
}

pub struct AtomicVec<T> {
    arr: Arc<Mutex<Vec<T>>>,
}

impl <T> AtomicVec<T> {
    pub fn with_capacity(capacity: usize) -> AtomicVec<T> {
        AtomicVec {
            arr: Arc::new(Mutex::new(Vec::with_capacity(capacity))),
        }
    }
}




enum HeapEntry<P> {
    Value(P),
    Transaction(usize),
}

pub struct Transaction<'a, P: 'a> {
    heap: &'a SharedHeap<P>,
    session: &'a Session<'a, P>,
    epoch: usize,
    // read_set
}


impl <'a, P> Transaction<'a, P> {
    pub fn borrow(&mut self, address:Address) -> &P {
        // in pessimistic, cas the address with a placeholder
        // add it to a list of things to return when done

        // in optimistic, don't cas it but
        // add it to a list of things to check when done
        
        unimplemented!()
    }

    pub fn insert(&mut self, value: P) -> Result<Address, P> {
        // find first empty slot
        // turn box into a pointer
        // swap it inside.
        Ok(1)
    }
    
    pub fn upsert<U: Fn(&mut P)>(&mut self, address: Address, value: P, on_conflict: U) -> Result<(),P> {
        unimplemented!()
    }

    pub fn delete(&mut self, address: Address) -> bool {
        unimplemented!()
    }

    pub fn copy(&mut self, address:Address) -> P {
        // same as read, but copying instead of borrowing
        unimplemented!()
    }

    pub fn replace(&mut self, address:Address, value:P) -> Result<(),P> {
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

struct Delete<P> {
    epoch: usize,
    address: Address,
    value: P,
}

pub struct Session<'a, P: 'a> {
    heap: &'a SharedHeap<P>,
    delete: Vec<Delete<P>>, // ArcMutex
    // read_set
} 

impl <'a, P> Session<'a, P> {
    fn collect_later(&mut self, epoch: usize, value: *mut P) {
        unimplemented!()
    }
    pub fn collect(&mut self) {
        // read current epoch, 
    }
    pub fn transaction<'b: 'a>(&'b self) -> Transaction<'b, P> {
        Transaction {
            heap: self.heap, 
            session: self, 
            epoch: self.heap.current_epoch(),
        } 
    }

}

impl<'a, T> Drop for Session<'a, T> {
    fn drop(&mut self) {
        // spin and collect ?
        ;
    }
}

pub struct SharedHeap<P> {
    table: AtomicVec<AtomicBox<HeapEntry<P>>>, // ArcMutex
    epoch: AtomicUsize,
}

impl <P> SharedHeap<P> {
    pub fn new(capacity: usize) -> SharedHeap<P> {
        let t = AtomicVec::with_capacity(capacity);
        let e = AtomicUsize::new(0);
        SharedHeap {
            table: t,
            epoch: e,
        }
    }

    fn current_epoch(&self) -> usize {
        self.epoch.load(Ordering::SeqCst)
    }

    pub fn session<'a>(&'a self) -> Session<'a, P> {
        let v = Vec::with_capacity(20);
        Session {heap:self, delete: v}
    }

}

#[cfg(test)]
mod tests {
    use SharedHeap;
    use std::sync::Arc;
    #[test]
    fn it_works() {
        let h = Arc::new(SharedHeap::new(1024));
        let mut s = h.session();
        let mut addr = 0;
        {
            let mut txn = s.transaction();
            addr = txn.insert("example".to_string()).unwrap();
        }
        {
            let mut txn = s.transaction();
            addr = txn.insert("example".to_string()).unwrap();
        }
        s.collect();
    }
}
