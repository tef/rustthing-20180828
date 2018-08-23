#![allow(unused_imports,dead_code,unused_variables,unused_mut,unused_assignments)]

use std::borrow::Borrow;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use std::ptr;
use std::mem;
use std::ops::Deref;
use std::fmt;
use std::marker::PhantomData;
use std::clone::Clone;
use std::slice;


// todo: make Descriptor use atomic state
//       conflicted state
//       Descriptor::race()
//       collector free list
//       heap free list
//       epoch advancement

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
        if index >= self.len.load(Ordering::SeqCst) {
            panic!("out of bounds");
        }
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
    // XXX as_mut_slice() instead
    pub fn as_slice(&mut self) -> &[*mut T] {
        unsafe { 
            let s = slice::from_raw_parts(self.ptr, self.len.load(Ordering::SeqCst));
            let s: &[*mut T] = mem::transmute(s);
            s
        }
    }
}


struct CAS<T> {index: usize, old: *mut T, new: *mut T}

#[derive(PartialEq, Copy, Clone)]
enum State { Reading = 0, Writing = 1 , Committing = 2, Committed = 3 , Cancelled = -1 }

struct Descriptor<T: std::clone::Clone> {
    state: State, // XXX Make Atomic
    operations: Vec<CAS<T>>,
}

impl <T: std::clone::Clone> Descriptor<T> {
    fn new() -> Descriptor<T> {
        let mut vec = Vec::with_capacity(8);
        Descriptor {
            state: State::Reading,
            operations: vec,
        }
    }
    fn add(&mut self, index: usize, old: *mut T, new: *mut T) {
        if self.state == State::Reading {
            self.state = State::Writing;
        } else if self.state != State::Writing {
            panic!("welp");
        }
        self.operations.push( 
            CAS {index: index, old: old, new: new}
        );
    }

    fn complete(&self) -> bool {
        self.state == State::Reading ||
        self.state == State::Cancelled ||
        self.state == State::Committed
    }

    fn committed(&self) -> bool {
        self.state == State::Committed
    }

    fn tagged_ptr(&self) -> *mut T {
        let ptr: *mut Descriptor<T> = unsafe { mem::transmute(self) };
        let ptr: usize = unsafe { mem::transmute(ptr) };
        let ptr = ptr | 1;
        unsafe{mem::transmute(ptr)}
    }

    fn apply(&mut self, vec: &AtomicPtrVec<T>) -> bool {
        if self.state == State::Committing || self.state == State::Cancelled {
            panic!("welp");
        }
        if self.state == State::Writing {
            let fake = self.tagged_ptr();
            for x in &self.operations {
                if x.old != fake {
                    print!("new\n");
                    let result = vec.compare_and_swap(x.index, x.old, x.new);
                    // XXX check for success, or race
                }
            }
            for x in &self.operations {
                print!("replace\n");
                let result = vec.compare_and_swap(x.index, x.old, x.new);
                // XXX check for success, or race
            }
            self.state = State::Committed;
        }
        self.state == State::Committed
    }

    fn cancel(&mut self, vec: &AtomicPtrVec<T>) {
        if self.state == State::Committing || self.state == State::Cancelled {
            panic!("welp");
        }
        if self.state == State::Writing {
            let fake = self.tagged_ptr();
            for x in &self.operations {
                if x.old == fake {
                    let result = vec.compare_and_swap(x.index, fake, ptr::null_mut());
                    // check for success, or race
                }
            }
        }
        self.state = State::Cancelled;
    }

    fn is_tagged(ptr: *mut T) -> bool {
        let ptr: usize = unsafe{mem::transmute(ptr)};
        ptr & 1 == 1
    }
    fn race(me: *mut Descriptor<T>, other: *mut Descriptor<T>) {
        // XXX
    }
}

pub struct Transaction<'a, 'b: 'a, P: 'a + 'b + std::clone::Clone> {
    vec: &'a AtomicPtrVec<P>,
    epoch: usize,
    descriptor: *mut Descriptor<P>,
    collector: &'a mut Collector<'b, P>,
}

impl <'a, 'b, P: std::clone::Clone> Transaction<'a, 'b, P> {
    pub fn apply(&mut self) -> bool {
        unsafe { (&mut *self.descriptor).apply(&self.vec) }
    }

    pub fn cancel(&mut self) {
        unsafe { (&mut *self.descriptor).cancel(&self.vec) }
    }

    fn load(&mut self, address: Address) -> *mut P {
        let mut flag = true;
        let mut ptr: *mut P = ptr::null_mut();
        while flag {
            ptr = self.vec.load(address);
            if !Descriptor::is_tagged(ptr) {
                flag = false;
            } else {
                unsafe {
                    let ptr: usize = mem::transmute(ptr);
                    let ptr: *mut Descriptor<P> = mem::transmute(ptr & (!1 as usize));
                    Descriptor::race(self.descriptor, ptr)
                }
            }
        }
        ptr
    }
          
    pub fn read(&mut self, address:Address) -> Option<&'a P> {
        let ptr = self.load(address);
        if ptr.is_null() || Descriptor::is_tagged(ptr) {
            None
        } else {
            unsafe { Some(&*ptr) }
        }
    }

    pub fn copy(&mut self, address:Address) -> Option<Box<P>> {
        let ptr = self.load(address);
        if ptr.is_null() || Descriptor::is_tagged(ptr) {
            None
        } else {
            unsafe { Some(Box::new((*ptr).clone())) }
        }
    }

    pub fn update(&mut self, address: Address, old: &'a P, new: Box<P>) {
        let new_ptr = Box::into_raw(new);
        unsafe {
            let d = &mut *self.descriptor;
            d.add(address, mem::transmute(old), new_ptr);
        }
    }
    
    pub fn insert(&mut self, value: Box<P>) -> Address {
        let ptr = Box::into_raw(value);
        unsafe {
            let d = &mut *self.descriptor;
            let fake = d.tagged_ptr();
            let addr = self.vec.append(fake).unwrap();
            d.add(addr, fake, ptr);
            addr
        }
    }
    

    pub fn delete(&mut self, address: Address) {
        let old = self.load(address);
        if Descriptor::is_tagged(old) {
            panic!("conflict")
        } else if !old.is_null() {
            unsafe {
                let d = &mut *self.descriptor;
                d.add(address, old, ptr::null_mut());
            } 
        }
    }

    pub fn overwrite(&mut self, address:Address, value:P) {
        let ptr = Box::into_raw(Box::new(value));
        let old = self.load(address);
        if Descriptor::is_tagged(old) {
            panic!("conflict")
        } else {
            unsafe {
                let d = &mut *self.descriptor;
                d.add(address, old, ptr);
            }
        }
    }


    pub fn upsert<U: Fn(&mut P)>(&mut self, address: Address, value: Box<P>, on_conflict: U) {
        let old = self.load(address);
        if old.is_null() {
            let ptr = Box::into_raw(value);
            unsafe {
                let d = &mut *self.descriptor;
                d.add(address, ptr::null_mut(), ptr);
            }
        } else if !Descriptor::is_tagged(old) { 
            let mut b = unsafe { Box::new( (*old).clone() ) };
            on_conflict(&mut b);
            let ptr = Box::into_raw(b);
            unsafe {
                let d = &mut *self.descriptor;
                d.add(address, old, ptr);
            }
        } else {
            panic!("conflict");
        }
    }

}

impl<'a, 'b, T: std::clone::Clone> Drop for Transaction<'a, 'b, T> {
    fn drop(&mut self) {
        unsafe {
            let d = &mut *self.descriptor;
            if !d.complete() {
                d.cancel(&self.vec);
            }
            let mut d = Box::from_raw(self.descriptor);
            if d.committed() {
                for op in &d.operations {
                    if !op.old.is_null() && !Descriptor::is_tagged(op.old) {
                        self.collector.collect_later(self.epoch, op.index, op.old);
                    }
                }
            } else {
                for op in &d.operations {
                    if !op.new.is_null() && !Descriptor::is_tagged(op.new) {
                        Box::from_raw(op.new);
                    }
                }

            }
        }
    }
}

struct Delete<P> {
    epoch: usize,
    address: Address,
    value: *mut P,
}

pub struct Collector<'a, P: 'a + std::clone::Clone> {
    heap: &'a AtomicHeap<P>,
    delete: Vec<Delete<P>>, // ArcMutex
    // read_set
} 

impl <'a, P: std::clone::Clone> Collector<'a, P> {
    fn collect_later(&mut self, epoch: usize, address: Address, value: *mut P) {
        self.delete.push(Delete{epoch: epoch, address: address, value: value});
    }
    pub fn collect(&mut self) {
        // read current epoch, scan list, prune
    }
    pub fn transaction<'b>(&'b mut self) -> Transaction<'b, 'a, P> {
        let d = Box::new(Descriptor::new());
        Transaction {
            vec: &self.heap.vec,
            epoch: self.heap.current_epoch(),
            descriptor: Box::into_raw(d),
            collector: self,
        } 
    }

}

impl<'a, T: std::clone::Clone> Drop for Collector<'a, T> {
    fn drop(&mut self) {
        // spin and collect ?
        // push onto heap's shared list
        ;
    }
}

pub struct AtomicHeap<P: std::clone::Clone> {
    vec: AtomicPtrVec<P>,
    epoch: AtomicUsize,
}

impl <P: std::clone::Clone> AtomicHeap<P> {
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
    pub fn transaction<'a, 'b>(&'a self, collector: &'a mut Collector<'b, P>) -> Transaction<'a, 'b, P> {
        let d = Box::new(Descriptor::new());
        Transaction {
            vec: &self.vec,
            epoch: self.current_epoch(),
            descriptor: Box::into_raw(d),
            collector: collector,
        } 
    }

}

impl<T: std::clone::Clone> Drop for AtomicHeap<T> {
    fn drop(&mut self) {
        for i in self.vec.as_slice() {
            if !i.is_null() {
                unsafe { Box::from_raw(*i) };
            }
        }
    }
}

pub struct Ref<'a, T:'a> {
    ptr: *mut T,
    _marker: PhantomData<&'a T>,
}

impl <'a, T> Deref for Ref<'a, T> {
    type Target = T;
    fn deref(&self) -> &T { unsafe { &*self.ptr} }
}

impl <'a, T> Borrow<T> for Ref<'a, T> {
    fn borrow(&self) -> &T { unsafe { &*self.ptr} }
}
impl <'a, T: fmt::Display> fmt::Display for Ref<'a, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(unsafe{&*self.ptr}, f)
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
            let mut txn = s.transaction();
            addr = txn.insert(Box::new("example".to_string()));
            txn.apply();
        }
        {
            let mut txn = s.transaction();
            let o = txn.read(addr).unwrap();
            print!("read: {}\n", o);
        }
        {
            let mut txn = s.transaction();
            let old = txn.read(addr).unwrap();
            let mut o = Box::new(old.clone());
            o.push_str(" mutated");
            txn.update(addr, old, o);
            txn.apply();
        }
        {
            let mut txn = s.transaction();
            let o = txn.read(addr).unwrap();
            print!("read: {}\n", o);
        }
        s.collect();
    }
}
