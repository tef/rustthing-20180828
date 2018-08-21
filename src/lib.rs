// feature(nll)]
#![allow(unused_imports,dead_code,unused_variables,unused_mut,unused_assignments)]

use std::borrow::Borrow;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use std::marker::PhantomData;
use std::ptr;
use std::mem;

type Address = usize;

//


#[derive(Debug)]
struct AtomicBox<T> {
    ptr: AtomicPtr<T>,
    _marker: PhantomData<T>,
}

impl<T> AtomicBox<T> {
    pub fn new(b: Box<T>) -> AtomicBox<T> {
        let uptr: usize = unsafe {  mem::transmute(Box::into_raw(b)) };
        // let uptr = uptr|1;
        let ptr: *mut T = unsafe { mem::transmute(uptr) };
        AtomicBox {
            ptr: AtomicPtr::new(ptr),
            _marker: PhantomData,
        }
    }

    pub fn load(&self) -> *mut T {
        let uptr: usize = unsafe { mem::transmute(self.ptr.load(Ordering::SeqCst)) };
        // let uptr = uptr & (!1 as usize);
        let ptr: *mut T = unsafe { mem::transmute(uptr) };
        ptr
    }   

    pub fn swap(&self, new: *mut T) -> *mut T {
        self.ptr.swap(new, Ordering::SeqCst)
    }

    pub fn compare_and_swap(&self, old: *mut T, new: *mut T) -> Result<*mut T, *mut T> {
        let r = self.ptr.compare_and_swap(old, new, Ordering::SeqCst);
        if old == r {
            Ok(r)
        } else {
            Err(r)
        }
    }

}
impl<T> Drop for AtomicBox<T> {
    fn drop(&mut self) {
        let p = self.ptr.swap(ptr::null_mut(), Ordering::SeqCst);
        if p != ptr::null_mut() {
            let uptr: usize = unsafe { mem::transmute(p)};
            if uptr & 1 == 0 {
                unsafe {Box::from_raw(p)};
            }
        }
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
    pub fn append(&self, value: T) -> usize {
        let mut v = self.arr.lock().unwrap();
        v.push(value);
        v.len()
    }
}



enum Operations<P> {
    Insert(Address, *mut P),
    CompareAndSwap(Address, *mut P, *mut P),
}

pub struct Transaction<'a, P: 'a> {
    heap: &'a SharedHeap<P>,
    epoch: usize,
    operations: Vec<Operations<P>>,
    // read_set
}


impl <'a, P> Transaction<'a, P> {
    pub fn borrow(&mut self, address:Address) -> &'a P {
        let mut vec = self.heap.table.lock().unwrap();
        let ab = vec.get(address).unwrap();
        let ptr = ab.load();
        let uptr: usize = unsafe { mem::transmute(ptr) };
        unsafe { & *ptr}
    }

    pub fn insert(&mut self, value: P) -> Address {
        let b = Box::new(value);
        let ab = AtomicBox::new(b);
        let ptr = ab.load();
        let mut vec = self.heap.table.lock().unwrap();
        let addr = vec.len();
        vec.push(ab);
        self.operations.push(Operations::Insert(addr, ptr));
        addr
    }
    

    pub fn delete(&mut self, address: Address) {
        unimplemented!()
    }

    pub fn copy(&mut self, address:Address) -> P {
        // same as read, but copying instead of borrowing
        unimplemented!()
    }

    pub fn replace(&mut self, address:Address, value:P) {
        // same as read, placeholder
        // list of things to write when done

        // optimisitic has write list & if read set promotes it
        unimplemented!()
    }
    pub fn apply<'b>(&mut self, session: &mut Session<'b,P>) -> bool {
        // in pessimistic, unlock each item and repl        ace it

        // in optimistic, lock each item, validate unchanged
        // return
        true
    }
    pub fn cancel(&mut self) -> bool {
        // race to undo
        true
    }
    pub fn upsert<U: Fn(&mut P)>(&mut self, address: Address, value: P, on_conflict: U) {
        // read, copy, update
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
    fn collect_later(&self, epoch: usize, value: *mut P) {
    }
    pub fn collect(&mut self) {
        // read current epoch, 
    }

}

impl<'a, T> Drop for Session<'a, T> {
    fn drop(&mut self) {
        // spin and collect ?
        ;
    }
}

pub struct SharedHeap<P> {
    table: Mutex<Vec<AtomicBox<P>>>, // ArcMutex
    epoch: AtomicUsize,
}

impl <P> SharedHeap<P> {
    pub fn new(capacity: usize) -> SharedHeap<P> {
        let t = Mutex::new(Vec::with_capacity(capacity));
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
    pub fn transaction<'a>(&'a self) -> Transaction<'a, P> {
        Transaction {
            heap: self,
            epoch: self.current_epoch(),
            operations: Vec::with_capacity(10),
        } 
    }

}

impl<T> Drop for SharedHeap<T> {
    fn drop(&mut self) {
        // 
    }
}

#[cfg(test)]
mod tests {
    use SharedHeap;
    use std::sync::Arc;
    use std::mem;
    #[test]
    fn it_works() {
        let h = Arc::new(SharedHeap::new(1024));
        let mut s = h.session();
        let mut addr = 0;
        {
            let mut txn = h.transaction();
            addr = txn.insert("example".to_string());
            txn.apply(&mut s);
        }
        print!("two \n");
        {
            let mut txn2 = h.transaction();
            print!("nice\n");
            let o = txn2.borrow(addr);
            print!("nice\n");
            let uptr: usize = unsafe {mem::transmute(o)};
            print!("two: {}\n", uptr);
        //       addr = txn2.insert("example2".to_string());
            print!("exit");
        }
        print!("nice");
        s.collect();
    }
}
