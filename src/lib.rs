// feature(nll)]
#![allow(unused_imports,dead_code,unused_variables,unused_mut,unused_assignments)]

use std::borrow::Borrow;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use std::ptr;
use std::mem;

type Address = usize;

struct AtomicPtrVec<T> {
    ptr: *mut AtomicPtr<T>,
    len: AtomicUsize,
    cap: usize,
}

impl <T> AtomicPtrVec<T> {
    pub fn new(size: usize) -> AtomicPtrVec<T> {
        let mut v: Vec<*mut ()> = vec![ptr::null_mut(); size];
        let mut v: Vec<AtomicPtr<T>> = unsafe { mem::transmute(v)};
        let a = AtomicPtrVec {
            ptr: v.as_mut_ptr(),
            len: AtomicUsize::new(0),
            cap: v.capacity(),
        };
        mem::forget(v);
        a
    }
    pub fn append(&self, value: *mut T) -> Result<usize, ()> {
        let index = self.len.fetch_add(1, Ordering::SeqCst);
        if index >= self.cap {
            panic!("out of bounds")
        }
        unsafe {
            let ptr = &*self.ptr.offset(index as isize);
            let old = ptr.compare_and_swap(ptr::null_mut(), value, Ordering::SeqCst);
            if old.is_null() {
                Ok(index)
            } else {
                Err(())
            }
        }
    }
    pub fn load(&self, index: usize) -> *mut T {
        unsafe {
            let ptr = &*self.ptr.offset(index as isize);
            ptr.load(Ordering::SeqCst)
        }
    }
    pub fn swap(&self, index: usize, new: *mut T) -> *mut T {
        unsafe {
            let ptr = &*self.ptr.offset(index as isize);
            ptr.swap(new, Ordering::SeqCst)
        }
    }

    pub fn compare_and_swap(&self, index: usize, old: *mut T, new: *mut T) -> Result<*mut T, *mut T> {
        unsafe {
            let ptr = &*self.ptr.offset(index as isize);
            let out = ptr.compare_and_swap(old, new, Ordering::SeqCst);
            if out == old {
                Ok(out)
            } else {
                Err(out)
            }
        }
    }
    pub unsafe fn to_vec(&mut self) -> Vec<*mut T> {
        let v = Vec::from_raw_parts(self.ptr, self.len.load(Ordering::SeqCst), self.cap);
        mem::transmute(v)
    }
}


struct CAS<T> {index: usize, old: *mut T, new: *mut T}

#[derive(PartialEq)]
enum State { Reading = 0, Uncommitted = 1 , Committing = 2, Committed = 3 , Cancelled = -1 }

struct Descriptor<T> {
    state: State, // Make Atomic
    operations: Vec<CAS<T>>,
}

impl <T> Descriptor<T> {
    fn new() -> Descriptor<T> {
        let mut vec = Vec::with_capacity(8);
        Descriptor {
            state: State::Reading,
            operations: vec,
        }
    }
    fn add(&mut self, index: usize, old: *mut T, new: *mut T) {
        self.operations.push( 
            CAS {index: index, old: old, new: new}
        );
    }
}

pub struct Transaction<'a, P: 'a> {
    vec: &'a AtomicPtrVec<P>,
    epoch: usize,
    descriptor: *mut Descriptor<P>,
    collector: &'a Collector<'a, P>,
}

struct Ref<'a, T:'a> {
    ptr: *'a mut T,
}

impl <'a,T> Deref for Ref<'a,T> {
    type Target = T;
    fn deref(&self) -> &T { unsafe { &*self.ptr} }
}
}

impl <'a, P> Transaction<'a, P> {
    fn fake_item(&self) -> *mut P {
        let ptr: usize = unsafe { mem::transmute(self.descriptor) };
        let ptr = ptr | 1;
        unsafe{mem::transmute(self.descriptor)}
    }
          
    pub fn borrow(&mut self, address:Address) -> &'a P {
        let ptr = self.vec.load(address);
        // check for fake record, race ...
        // check for null pointer, panic
        unsafe { &*ptr }
    }
    
    pub fn insert(&mut self, value: P) -> Address {
        let ptr = Box::into_raw(Box::new(value));
        let fake = self.fake_item();
        let addr = self.vec.append(fake).unwrap();
        unsafe {
            let d = &mut *self.descriptor;
            if d.state != State::Reading && d.state != State::Committing {
                panic!("welp");
            }
            d.state = State::Committing;
            d.add(addr, fake, ptr);
        }
        addr
    }
    

    pub fn delete(&mut self, address: Address) {
        unimplemented!()
    }

    pub fn copy(&mut self, address:Address) -> P {
        // same as read, but copying instead of borrowing
        unimplemented!()
    }

    pub fn upsert<U: Fn(&mut P)>(&mut self, address: Address, value: P, on_conflict: U) {
        // read, copy, update
        unimplemented!()
    }

    pub fn overwrite(&mut self, address:Address, value:P) {
        // same as read, placeholder
        // list of things to write when done

        // optimisitic has write list & if read set promotes it
        unimplemented!()
    }
    pub fn apply(&mut self) -> bool {
        unsafe {
            let d = &mut *self.descriptor;
            if d.state == State::Cancelled {
                panic!("Cancelled!");
            }
            if d.state == State::Uncommitted {
                d.state = State::Committed;
                let fake = self.fake_item();
                for x in &d.operations {
                    if x.old != fake {
                        let result = self.vec.compare_and_swap(x.index, x.old, x.new);
                        // check for success, or race
                    }
                }
                for x in &d.operations {
                    let result = self.vec.compare_and_swap(x.index, x.old, x.new);
                    // check for success, or race
                }
            }
            d.state = State::Committed;
            // free / collect old values
        }
        true
    }

    pub fn cancel(&mut self) -> bool {
        unsafe {
            let d = &mut *self.descriptor;
            d.state = State::Cancelled;
            // free inserted records
        }
        true
    }

}

impl<'a, T> Drop for Transaction<'a, T> {
    fn drop(&mut self) {
        // if !apply, cancel
        unsafe { Box::from_raw(self.descriptor) };
    }
}

struct Delete<P> {
    epoch: usize,
    address: Address,
    value: *mut P,
}

pub struct Collector<'a, P: 'a> {
    heap: &'a AtomicHeap<P>,
    delete: Vec<Delete<P>>, // ArcMutex
    // read_set
} 

impl <'a, P> Collector<'a, P> {
    fn collect_later(&mut self, epoch: usize, address: Address, value: *mut P) {
        self.delete.push(Delete{epoch: epoch, address: address, value: value});
    }
    pub fn collect(&mut self) {
        // read current epoch, scan list, prune
    }

}

impl<'a, T> Drop for Collector<'a, T> {
    fn drop(&mut self) {
        // spin and collect ?
        ;
    }
}

pub struct AtomicHeap<P> {
    vec: AtomicPtrVec<P>,
    epoch: AtomicUsize,
}

impl <P> AtomicHeap<P> {
    pub fn new(capacity: usize) -> AtomicHeap<P> {
        let t = AtomicPtrVec::new(capacity);
        let e = AtomicUsize::new(0);
        AtomicHeap {
            vec: t,
            epoch: e,
        }
    }

    fn current_epoch(&self) -> usize {
        self.epoch.load(Ordering::SeqCst)
    }

    pub fn collector<'a>(&'a self) -> Collector<'a, P> {
        let v = Vec::with_capacity(20);
        Collector {heap:self, delete: v}
    }
    pub fn transaction<'a, 'b>(&'a self, collector: &'a mut Collector<'b, P>) -> Transaction<'a, P> {
        let d = Box::new(Descriptor::new());
        Transaction {
            vec: &self.vec,
            epoch: self.current_epoch(),
            descriptor: Box::into_raw(d),
            collector: collector,
        } 
    }

}

impl<T> Drop for AtomicHeap<T> {
    fn drop(&mut self) {
        let v = unsafe { self.vec.to_vec() };
        for i in v {
            if !i.is_null() {
                unsafe { Box::from_raw(i) };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use AtomicHeap;
    use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::mem;
    use std::ptr;
    #[test]
    fn it_works() {
        let h = Arc::new(AtomicHeap::new(1024));
        let mut s = h.collector();
        let mut addr = 0;
        {
            let mut txn = h.transaction(&mut s);
            addr = txn.insert("example".to_string());
            txn.apply();
        }
        {
            let mut txn2 = h.transaction(&mut s);
            let o = txn2.borrow(addr);
            print!("read: {}\n", o);
        }
        s.collect();
    }
}
