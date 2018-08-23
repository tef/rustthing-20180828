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


// todo
//      collector has 
//      refcount of active/quiet sessions + increment epoch when matching
//
//      descriptor uses atomicusize for state
//      add conflicted state, auto cancels, maybe Result<>/bool?
//      Descriptor::race()
//
//      global free list is atomicdeque 
//          with push/pop left      
//      insert uses list of empty addresses
//            one in session, one in heap
//
//      inline
//      tests
//
//      trie?

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
    unsafe fn race(me: *mut Descriptor<T>, other: *mut T) {
        let ptr: usize = mem::transmute(other);
        let ptr: *mut Descriptor<T> = mem::transmute(ptr & (!1 as usize));
        panic!("no race");
    }
}

pub struct Ref<'a, T:'a> { // make cow
    address: Address,
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

pub struct Transaction<'a, 'b: 'a, P: 'a + 'b + std::clone::Clone> {
    vec: &'a AtomicPtrVec<P>,
    collector: &'a Collector,
    session: &'a mut Session<'b, P>,
    descriptor: *mut Descriptor<P>,
}

impl <'a, 'b, P: std::clone::Clone> Transaction<'a, 'b, P> {
    fn new(heap: &'a Heap<P>, session: &'a mut Session<'b, P>) -> Transaction<'a, 'b, P> {
        let d = Box::new(Descriptor::new());
        unsafe {session.enter_critical()};
        Transaction {
            vec: &heap.vec,
            collector: &heap.collector,
            session: session,
            descriptor: Box::into_raw(d),
        } 
    }
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
                    Descriptor::race(self.descriptor, ptr)
                }
            }
        }
        ptr
    }
          
    pub fn read(&mut self, address:Address) -> Ref<'a, P> {
        let ptr = self.load(address);
        Ref{ address: address, ptr: ptr, _marker: PhantomData } 
    }

    pub fn update(&mut self, old: Ref<'a, P>, new: Box<P>) {
        let new_ptr = Box::into_raw(new);
        unsafe {
            let d = &mut *self.descriptor;
            d.add(old.address, mem::transmute(old.ptr), new_ptr);
        }
    }

    pub fn borrow(&mut self, address:Address) -> Option<&'a P> {
        let ptr = self.load(address);
        if ptr.is_null() {
            None
        } else {
            unsafe { Some(&*ptr) }
        }
    }

    pub fn compare_and_swap(&mut self, address: Address, old: &'a P, new: Box<P>) {
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
        if !old.is_null() {
            unsafe {
                let d = &mut *self.descriptor;
                d.add(address, old, ptr::null_mut());
            } 
        }
    }

    pub fn overwrite(&mut self, address:Address, value:P) {
        let ptr = Box::into_raw(Box::new(value));
        let old = self.load(address);
        unsafe {
            let d = &mut *self.descriptor;
            d.add(address, old, ptr);
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
                let epoch = self.collector.current_epoch();
                for op in &d.operations {
                    if !op.old.is_null() && !Descriptor::is_tagged(op.old) {
                        self.session.collect_later(epoch, op.index, op.old);
                    }
                }
            } else {
                for op in &d.operations {
                    if !op.new.is_null() && !Descriptor::is_tagged(op.new) {
                        Box::from_raw(op.new);
                    }
                }

            }
            self.session.exit_critical();
        }
    }
}

struct Delete<P> {
    epoch: usize,
    address: Address,
    value: *mut P,
}

#[derive(PartialEq)]
enum SessionState {
    Inactive,
    Active,
    Quiet(usize),
}

pub struct Session<'a, P: 'a + std::clone::Clone> {
    heap: &'a Heap<P>,
    collector: &'a Collector,
    delete: Option<Vec<Delete<P>>>, 
    state: SessionState,
    clear: bool,
    behaviour: SessionBehaviour,
} 

#[derive(PartialEq)]
enum SessionBehaviour {
    ClearOnExit, CloseOnExit, ClearOnExitCloseOnDelete
}

impl <'a, P: std::clone::Clone> Session<'a, P> {
    fn new(heap: &'a Heap<P>, behaviour: SessionBehaviour) -> Session<'a, P> {
        let c: bool = match behaviour {
            SessionBehaviour::ClearOnExit => true,
            SessionBehaviour::CloseOnExit => false,
            SessionBehaviour::ClearOnExitCloseOnDelete => true,
        };
        let v = Vec::with_capacity(20);
        Session {
            heap: heap, 
            collector: &heap.collector,
            delete: Some(v), 
            state: SessionState::Inactive, 
            clear: c,
            behaviour: behaviour,
        }
    }
    unsafe fn enter_critical(&mut self) {
        self.collect();
        let c: bool = match self.behaviour {
            SessionBehaviour::ClearOnExit => true,
            SessionBehaviour::CloseOnExit => false,
            SessionBehaviour::ClearOnExitCloseOnDelete => true,
        };
        if self.state == SessionState::Inactive {
            self.state = self.collector.enter_session();
            self.clear = c;
        }
    }
    unsafe fn exit_critical(&mut self) {
        if self.clear {
            self.state = self.collector.clear_session(&self.state);
        } else {
            self.state = self.collector.exit_session(&self.state);
        }
        self.collect();
    }

    unsafe fn collect_later(&mut self, epoch: usize, address: Address, value: *mut P) {
        if self.behaviour == SessionBehaviour::ClearOnExitCloseOnDelete {
            self.clear = false;
        }
        self.delete.as_mut().unwrap().push(Delete{epoch: epoch, address: address, value: value});
    }

    pub fn collect(&mut self) {
        self.heap.collect();
        let epoch = self.collector.current_epoch();
        // XXX
        // peek at vec, clean any that are done
        // for x in delete, delete if epoch-2 ...
    }
    pub fn transaction<'b>(&'b mut self) -> Transaction<'b, 'a, P> {
        Transaction::new(self.heap, self)
    }

}

impl<'a, T: std::clone::Clone> Drop for Session<'a, T> {
    fn drop(&mut self) {
        unsafe {
            if self.state != SessionState::Inactive {
                self.collector.exit_session(&self.state);
            }
        }
        self.collect();
        self.heap.collect_later(&mut self.delete.take().unwrap());
    }
}


struct Collector {
    epoch: AtomicUsize
}

impl Collector {
    fn current_epoch(&self) -> usize {
        self.epoch.load(Ordering::SeqCst)
    }

    fn advance(&self) -> usize {
        self.epoch.load(Ordering::SeqCst)
    }

    unsafe fn enter_session(&self) -> SessionState {
        // incr active and advance
        return SessionState::Active
    }

    unsafe fn clear_session(&self, state: &SessionState) -> SessionState{
        let epoch = self.current_epoch();
        match state {
            SessionState::Inactive => panic!{"sync!"},
            SessionState::Active => {
                // incr quiet or advance
                
            },
            SessionState::Quiet(e) => {
                if *e != epoch {
                  //  incr quiet or advance
                } else {
                    // advance
                }
            }
        };
        let epoch = self.current_epoch();
        SessionState::Quiet(epoch)
    }

    unsafe fn exit_session(&self, state: &SessionState) -> SessionState {
        match state {
            SessionState::Inactive => {},
            SessionState::Active => {
                // decr active
                
            },
            SessionState::Quiet(e) =>  {
                //decr quiet 
            }
        };
        // decr active
        SessionState::Inactive
        // check if currently active, etc
    }

}


pub struct Heap<P: std::clone::Clone> {
    vec: AtomicPtrVec<P>,
    collector: Collector,
    delete: Mutex<Vec<Delete<P>>>, // Make atomic
}

impl <P: std::clone::Clone> Heap<P> {
    pub fn new(capacity: usize) -> Heap<P> {
        let t = AtomicPtrVec::new(capacity);
        let e = AtomicUsize::new(0);
        let d = Vec::with_capacity(100);
        Heap {
            vec: t,
            collector: Collector { epoch:e, },
            delete: Mutex::new(d)
        }
    }

    pub fn session<'a>(&'a self) -> Session<'a, P> {
        Session::new(self, SessionBehaviour::ClearOnExit)
    }

    fn epoch(&self) -> usize {
        self.collector.current_epoch()
    }

    fn collect_later(&self, delete: &mut Vec<Delete<P>>) {
        let mut v = self.delete.lock().unwrap();
        v.append(delete);
    }

    pub fn collect(&self) {
        let epoch = self.collector.advance();
        // check for free list
        // XXX
    }

}

impl<T: std::clone::Clone> Drop for Heap<T> {
    fn drop(&mut self) {
        // todo self.collect();
        for i in self.vec.as_slice() {
            if !i.is_null() {
                unsafe { Box::from_raw(*i) };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use Heap;
    use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::mem;
    use std::ptr;
    #[test]
    fn it_works() {
        let h = Arc::new(Heap::new(1024));
        let mut s = h.session();
        let mut addr = 0;
        {
            let mut txn = s.transaction();
            addr = txn.insert(Box::new("example".to_string()));
            txn.apply();
        }
        {
            let mut txn = s.transaction();
            let o = txn.borrow(addr).unwrap();
            print!("read: {}\n", o);
        }
        {
            let mut txn = s.transaction();
            let old = txn.borrow(addr).unwrap();
            let mut o = Box::new(old.clone());
            o.push_str(" mutated");
            txn.compare_and_swap(addr, old, o);
            txn.apply();
        }
        {
            let mut txn = s.transaction();
            let o = txn.borrow(addr).unwrap();
            print!("read: {}\n", o);
        }
        {
            let mut txn = s.transaction();
            let old = txn.borrow(addr).unwrap();
            let mut o = Box::new(old.clone());
            o.push_str(" cancelled");
            txn.compare_and_swap(addr, old, o);
            txn.cancel();
        }
        {
            let mut txn = s.transaction();
            let o = txn.borrow(addr).unwrap();
            print!("read: {}\n", o);
        }
        s.collect();
    }
    #[test]
    fn test2() {
        let h = Arc::new(Heap::new(1024));
        let mut s = h.session();
        let mut addr = 0;
        {
            let mut txn = s.transaction();
            addr = txn.insert(Box::new("example".to_string()));
            txn.apply();
        }
        {
            let mut txn = s.transaction();
            let o = txn.read(addr);
            print!("read: {}\n", o);
        }
        {
            let mut txn = s.transaction();
            let old = txn.read(addr);
            let mut new = Box::new(old.clone());
            new.push_str(" mutated");
            txn.update(old, new);
            txn.apply();
        }
        {
            let mut txn = s.transaction();
            let o = txn.read(addr);
            print!("read: {}\n", o);
        }
        {
            let mut txn = s.transaction();
            txn.delete(addr);
            txn.apply();
        }
    }
}
