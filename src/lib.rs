use std::borrow::Borrow;

type Address = usize;


pub struct AtomicHeap<P> {
    arr: Vec<P>, // ArcMutex
    // either cells or
}

impl <P> AtomicHeap<P> {
    pub fn new(capacity: usize) -> AtomicHeap<P> {
        let v = Vec::with_capacity(capacity);
        AtomicHeap {
            arr: v,
        }
    }

    pub fn collector<'a>(&'a self) -> Collector<'a, P> {
        Collector{table:self}
    }

    pub fn transaction<'a>(&'a self, collector: &'a Collector<'a, P>) -> Transaction<'a, P> {
        Transaction {table:self, collector: collector} 
    }

}

pub struct Collector<'a, P: 'a> {
    table: &'a AtomicHeap<P>,
    // read_set
} 
impl <'a, P> Collector<'a, P> {
    fn collect_later(&self, epoch: usize, value: *mut P) {
        unimplemented!()
    }
    pub fn collect(&mut self) {
        // read current epoch, 
        unimplemented!()
    }
}

impl<'a, T> Drop for Collector<'a, T> {
    fn drop(&mut self) {
        // spin and collect ?
        ;
    }
}

pub struct Transaction<'a, P: 'a> {
    table: &'a AtomicHeap<P>,
    collector: &'a Collector<'a, P>,
    // read_set
}


impl <'a, P> Transaction<'a, P> {
    pub fn insert(&mut self, value: P) -> Result<Address, P> {
        // find first empty slot
        // turn box into a pointer
        // swap it inside.
        unimplemented!()
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
    use AtomicHeap;
    use std::sync::Arc;
    #[test]
    fn it_works() {
        let h = Arc::new(AtomicHeap::new(1024));
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
