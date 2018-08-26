#![allow(unused_imports,dead_code,unused_variables,unused_mut,unused_assignments)]

use std::borrow::Borrow;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicPtr, AtomicUsize, AtomicIsize, Ordering};
use std::ptr;
use std::mem;
use std::ops::Deref;
use std::fmt;
use std::marker::PhantomData;
use std::clone::Clone;
use std::slice;


// todo
//      on read, racing to commit in Descriptor::race()
//      
//      remove locks from heap, session delete
//      list is atomicdeque with push/pop left      
//
//      session, heap have lists of empty addresses
//      insert uses list of empty addresses
//      
//      epoch/active overflow
//      inline/annotations, tests, docs
//
//      then, trie?

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

    #[inline]
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
    #[inline]
    pub fn load(&self, index: usize) -> *mut T {
        if index >= self.len.load(Ordering::SeqCst) {
            panic!("out of bounds");
        }
        unsafe {
            let ptr = &*self.ptr.offset(index as isize);
            ptr.load(Ordering::SeqCst)
        }
    }
    #[inline]
    pub fn swap(&self, index: usize, new: *mut T) -> *mut T {
        unsafe {
            let ptr = &*self.ptr.offset(index as isize);
            ptr.swap(new, Ordering::SeqCst)
        }
    }

    #[inline]
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
enum State { Reading = 0, Writing = 1 , Preparing =2, Committing = 3, Committed = 4 , Cancelled = -1, Conflicted = -2 }

struct Descriptor<T> {
    _state: AtomicIsize,
    operations: Vec<CAS<T>>,
}

impl <T> Descriptor<T> {
    fn new() -> Descriptor<T> {
        let mut vec = Vec::with_capacity(8);
        Descriptor {
            _state: AtomicIsize::new(State::Reading as isize),
            operations: vec,
        }
    }
    #[inline]
    fn state(&self) -> State {
        let i = self._state.load(Ordering::Relaxed);
        match i {
            0 => State::Reading,
            1 => State::Writing,
            2 => State::Preparing,
            3 => State::Committing,
            4 => State::Committed,
            -1 => State::Cancelled,
            -2 => State::Conflicted,
            _ => panic!("state"),
        }
    }
    #[inline]
    fn set_state(&self, state: State, ordering: Ordering) {
        self._state.store(state as isize, ordering);
    }
    #[inline]
    fn add(&mut self, index: usize, old: *mut T, new: *mut T) {
        if self.state() == State::Reading {
            self.set_state(State::Writing, Ordering::Relaxed);
        } else if self.state() != State::Writing {
            panic!("welp");
        }
        self.operations.push( 
            CAS {index: index, old: old, new: new}
        );
    }

    #[inline]
    fn complete(&self) -> bool {
        let state = self.state();
        state == State::Reading ||
        state == State::Committed ||
        state == State::Cancelled ||
        state == State::Conflicted
    }

    #[inline]
    fn committed(&self) -> bool {
        self.state() == State::Committed
    }

    #[inline]
    fn tagged_ptr(&self) -> *mut T {
        let ptr: *mut Descriptor<T> = unsafe { mem::transmute(self) };
        let ptr: usize = unsafe { mem::transmute(ptr) };
        let ptr = ptr | 1;
        unsafe{mem::transmute(ptr)}
    }

    #[inline]
    fn apply(&mut self, vec: &AtomicPtrVec<T>) -> bool {
        let state = self.state();
        if state == State::Reading {
            self.set_state(State::Committed, Ordering::Relaxed);
            true
        } else if state == State::Writing {
            let mut conflict = false;
            let mut changed = 0;
            self.set_state(State::Preparing, Ordering::SeqCst);
            let fake = self.tagged_ptr();
            // swap in descriptor, inserts already placed
            for x in &self.operations {
                if x.old != fake {
                    let result = vec.compare_and_swap(x.index, x.old, fake);
                    if result.is_ok() {
                        changed +=1;
                    } else {
                        conflict = true;
                        break;
                    }
                } else { 
                    changed += 1;
                }
            }
            if conflict {
                self.set_state(State::Conflicted, Ordering::SeqCst);
                while changed > 0 {
                    changed -=1;
                    let x = self.operations.get(changed).unwrap();
                    if x.old != fake {
                        let result = vec.compare_and_swap(x.index, fake, x.old);
                        if result.is_err() { panic!("sync") }
                    } else {
                        let result = vec.compare_and_swap(x.index, fake, ptr::null_mut());
                        if result.is_err() { panic!("sync") }
                    }
                }
                false
            } else {
                self.set_state(State::Committing, Ordering::SeqCst);
                for x in &self.operations {
                    let result = vec.compare_and_swap(x.index, fake, x.new);
                    if result.is_err() { panic!("sync") }
                }
                self.set_state(State::Committed, Ordering::SeqCst);
                true
            }
        } else if state == State::Committed {
            true
        } else if state == State::Cancelled || state == State::Conflicted {
            false
        } else {
            panic!("bad state");
        }
    }

    #[inline]
    fn cancel(&mut self, vec: &AtomicPtrVec<T>) {
        let state = self.state();
        if state == State::Writing {
            let fake = self.tagged_ptr();
            for x in &self.operations {
                if x.old == fake {
                    let result = vec.compare_and_swap(x.index, fake, ptr::null_mut());
                    // check for success, or race
                }
            }
            self.set_state(State::Cancelled, Ordering::SeqCst);
        }
    }

    #[inline]
    fn is_tagged(ptr: *mut T) -> bool {
        let ptr: usize = unsafe{mem::transmute(ptr)};
        ptr & 1 == 1
    }
    #[inline]
    unsafe fn race(me: *mut Descriptor<T>, other: *mut T) {
        let ptr: usize = mem::transmute(other);
        let ptr: *mut Descriptor<T> = mem::transmute(ptr & (!1 as usize));
        // panic!("no race");
    }
}

pub struct Ref<'a, T:'a> { // make cow
    address: Address,
    ptr: *mut T,
    _marker: PhantomData<&'a T>,
}

impl <'a, T: std::clone::Clone> Ref<'a, T> {
    pub fn copy(&self) -> Box<T> {
        unsafe {
            Box::new((&*self.ptr).clone())
        }
    }
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

pub struct Transaction<'a, 'b: 'a, P: 'a + 'b>  {
    vec: &'a AtomicPtrVec<P>,
    collector: &'a Collector<P>,
    session: &'a mut Session<'b, P>,
    descriptor: *mut Descriptor<P>,
}

impl <'a, 'b, P> Transaction<'a, 'b, P> {
    fn new(heap: &'a Heap<P>, session: &'a mut Session<'b, P>) -> Transaction<'a, 'b, P> {
        let d = Box::new(Descriptor::new());
        unsafe {session.start_transaction()};
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
          
    pub fn read(&mut self, address:Address) -> Option<Ref<'a, P>> {
        let ptr = self.load(address);
        if ptr.is_null() {
            None
        } else {
            Some(Ref{ address: address, ptr: ptr, _marker: PhantomData })
        }
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

}

impl<'a, 'b, T> Drop for Transaction<'a, 'b, T> {
    fn drop(&mut self) {
        unsafe {
            let d = &mut *self.descriptor;
            if !d.complete() {
                d.cancel(&self.vec);
            }
            let mut d = Box::from_raw(self.descriptor);
            if d.committed() {
                let epoch = self.collector.epoch();
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
            self.session.exit_transaction();
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

pub struct Session<'a, P: 'a> {
    heap: &'a Heap<P>,
    collector: &'a Collector<P>,
    delete: Vec<Delete<P>>, 
    state: SessionState,
    clear: bool,
    behaviour: SessionBehaviour,
} 

#[derive(PartialEq)]
enum SessionBehaviour {
    ClearOnExit, CloseOnExit, ClearOnExitCloseOnDelete
}

impl <'a, P> Session<'a, P> {
    fn new(heap: &'a Heap<P>, behaviour: SessionBehaviour) -> Session<'a, P> {
        let c: bool = match behaviour {
            SessionBehaviour::ClearOnExit => true,
            SessionBehaviour::CloseOnExit => false,
            SessionBehaviour::ClearOnExitCloseOnDelete => true,
        };
        let v = Vec::with_capacity(100);
        Session {
            heap: heap, 
            collector: &heap.collector,
            delete: v, 
            state: SessionState::Inactive, 
            clear: c,
            behaviour: behaviour,
        }
    }

    pub fn transaction<'b>(&'b mut self) -> Transaction<'b, 'a, P> {
        Transaction::new(self.heap, self)
    }

    #[inline]
    unsafe fn start_transaction(&mut self) {
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
    #[inline]
    unsafe fn exit_transaction(&mut self) {
        if self.clear {
            self.state = self.collector.clear_session(&self.state);
            self.collect();
        } else {
            self.state = self.collector.exit_session(&self.state);
            self.collector.collect_later(&mut self.delete);
        }
    }

    #[inline]
    unsafe fn collect_later(&mut self, epoch: usize, address: Address, value: *mut P) {
        if self.behaviour == SessionBehaviour::ClearOnExitCloseOnDelete {
            self.clear = false;
        }
        self.delete.push(Delete{epoch: epoch, address: address, value: value});
    }

    pub fn collect(&mut self) {
        let epoch = self.collector.epoch();
        let mut i = 0;
        while i < self.delete.len() {
            let e = self.delete.get(i).unwrap().epoch;
            if e+2 <= epoch {
                self.delete.remove(i);
            } else {
                break;
            }
        }
    }
}

impl<'a, T> Drop for Session<'a, T> {
    fn drop(&mut self) {
        unsafe {
            if self.state != SessionState::Inactive {
                self.collector.exit_session(&self.state);
            }
        }
        self.collect();
        self.collector.collect_later(&mut self.delete);
    }
}


struct Collector<P> {
    state: AtomicUsize,
    delete: Mutex<Vec<Delete<P>>>, // Make atomic
}

impl<P> Collector<P> {
    #[inline]
    fn collect_later(&self, delete: &mut Vec<Delete<P>>) {
        let mut v = self.delete.lock().unwrap();
        v.append(delete);
    }

    pub fn collect(&self) {
        let epoch = self.epoch();
        let mut i = 0;
        let mut v = self.delete.lock().unwrap();
        while i < v.len() {
            let e = v.get(i).unwrap().epoch;
            if e+2 <= epoch {
                v.remove(i);
            } else {
                break;
            }
        }
    }

    #[inline]
    fn epoch(&self) -> usize {
        self.state.load(Ordering::SeqCst) & 0xFFFF
    }

    #[inline]
    unsafe fn incr_active(&self) {
        loop {
            let state: usize  = self.state.load(Ordering::SeqCst); 
            let epoch = state & 0x0000_0000_FFFF_FFFF;
            let mut active = (state & 0x0000_FFFF_0000_0000) >> 32;
            let quiet = (state & 0xFFFF_0000_0000_0000) >> 48;
            active +=1;
            let new = (quiet << 48) | (active << 32) | epoch;
            let result = self.state.compare_and_swap(state, new, Ordering::SeqCst);
            if result == state {
                break
            }
        }
    }

    #[inline]
    unsafe fn set_quiet(&self, quiet_epoch: Option<usize>) -> (bool, usize) {
        let mut epoch = 0;
        let mut new_epoch = false;
        loop {
            let state = self.state.load(Ordering::SeqCst); 
            epoch = state & 0x0000_0000_FFFF_FFFF;
            let active = (state & 0x0000_FFFF_0000_0000) >> 32;
            let mut quiet = (state & 0xFFFF_0000_0000_0000) >> 48;
            new_epoch = false;
            if quiet_epoch.is_none() || quiet_epoch.unwrap() != epoch {
                quiet +=1;
                if quiet == active {
                    quiet = 0;
                    epoch +=1;
                    new_epoch = true
                }
            } else {
                break
            }
            let new = (quiet << 48) | (active << 32) | epoch;
            let result = self.state.compare_and_swap(state, new, Ordering::SeqCst);
            if result == state {
                break
            }
        }
        (new_epoch, epoch)
    }

    #[inline]
    unsafe fn decr_active(&self, quiet_epoch: Option<usize>) -> (bool, usize) {
        let mut epoch = 0;
        let mut new_epoch = false;
        loop {
            let state = self.state.load(Ordering::SeqCst);
            epoch = state & 0x0000_0000_FFFF_FFFF;
            let mut active = (state & 0x0000_FFFF_0000_0000) >> 32;
            let mut quiet = (state & 0xFFFF_0000_0000_0000) >> 48;
            new_epoch = false;
            if quiet_epoch.is_some() && quiet_epoch.unwrap() == epoch {
                active -=1;
                quiet -=1;
            } else {
                active-=1;
            }
            if quiet == active {
                quiet = 0;
                epoch +=1;
                new_epoch = true;
            }
            let new = (quiet << 48) | (active << 32) | epoch;
            let result = self.state.compare_and_swap(state, new, Ordering::SeqCst);
            if result == state {
                break
            }
        }
        (new_epoch, epoch)
    }

    #[inline]
    unsafe fn enter_session(&self) -> SessionState {
        self.incr_active();
        SessionState::Active
    }

    #[inline]
    unsafe fn clear_session(&self, state: &SessionState) -> SessionState{
        match state {
            SessionState::Inactive => { panic!("sync!") },
            SessionState::Active => { 
                let (new_epoch, epoch) = self.set_quiet(None);
                if new_epoch {
                    SessionState::Active
                } else { 
                    SessionState::Quiet(epoch)
                }
            },
            SessionState::Quiet(e) => {
                let (new_epoch, epoch) = self.set_quiet(Some(*e));
                if new_epoch {
                    SessionState::Active
                } else { 
                    SessionState::Quiet(epoch)
                }
            },
        }
    }

    #[inline]
    unsafe fn exit_session(&self, state: &SessionState) -> SessionState {
        match state {
            SessionState::Inactive => {},
            SessionState::Active => { self.decr_active(None); },
            SessionState::Quiet(e) =>  { self.decr_active(Some(*e)); },
        };
        SessionState::Inactive
    }

}

impl<T> Drop for Collector<T> {
    fn drop(&mut self) {
        let mut v = self.delete.lock().unwrap();
        v.clear();
    }
}

pub struct Heap<P> {
    vec: AtomicPtrVec<P>,
    collector: Collector<P>,
}

impl <P> Heap<P> {
    pub fn new(capacity: usize) -> Heap<P> {
        let t = AtomicPtrVec::new(capacity);
        let e = AtomicUsize::new(0);
        let d = Vec::with_capacity(100);
        Heap {
            vec: t,
            collector: Collector { state: e, delete: Mutex::new(d) },
        }
    }

    pub fn session<'a>(&'a self) -> Session<'a, P> {
        self.collector.collect();
        Session::new(self, SessionBehaviour::ClearOnExit)
    }

    pub fn collect(&self) {
        self.collector.collect()
    }

}
unsafe impl <T> Send for Heap<T> {}
unsafe impl <T> Sync for Heap<T> {}

impl<T> Drop for Heap<T> {
    fn drop(&mut self) {
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
    use std::sync::{Arc, Mutex};
    use std::mem;
    use std::ptr;
    use std::thread;
    #[test]
    fn it_works() {
        let h = Arc::new(Heap::new(1024));
        let mut s1 = h.session();
        let mut txn1 = s1.transaction();
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
            assert!(o == "example");
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
            assert!(o == "example mutated");
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
            assert!(o == "example mutated");
        }
        s.collect();
    }
    #[test]
    fn threadtest() {
        let nitems = 5000;
        let nthreads = 250;
        let h = Arc::new(Heap::<usize>::new(nitems));
        { 
            let mut s = h.session();
            let mut t = s.transaction();
            for i in 0..nitems {
                t.insert(Box::new(0));
            };
            t.apply();
        }
        let mut threads = vec![];
        for t in 0..nthreads {
            let o  = nitems/nthreads * t;
            let bh = h.clone();
            let t = thread::spawn(move || {
                for i in 0..nitems {
                    let mut s = bh.session();
                    loop {
                        let j = (i+o) % nitems;
                        //print!("thread: {}, item: {}, \n", t,  j);
                        let mut tx = s.transaction();
                        let n = tx.borrow(j).unwrap();
                        //print!("thread: {}, item: {}, val {} \n", t,  j, n);
                        tx.compare_and_swap(j, n, Box::new(n+1));
                        if tx.apply() { break }
                        // print!("r");
                    }; 
                    //print!(".");
                }
            });
            threads.push(t);
            for k in 0..4 {

                let bh = h.clone();
                let t = thread::spawn(move || {
                    let mut s = bh.session();
                    for i in 0..3 { 
                        for i in 0..nitems {
                            let mut tx = s.transaction();
                            let n = tx.borrow(i).unwrap();
                        }
                    };
                });
                threads.push(t);
            }
        };
        for t in threads {
            let _ = t.join();
        };
        { 
            let mut s = h.session();
            let mut t = s.transaction();
            for i in 0..nitems {
                let v = t.borrow(i).unwrap();
                assert!(*v == nthreads);
            };
        }
    }

    #[test]
    fn vectest() {
        let nitems = 5000;
        let nthreads = 250;

        let mut v = Vec::with_capacity(nitems);
        for i in 0..nitems {
            v.push(Box::new(0));
        };
        let h = Arc::new(Mutex::new(v));
        let mut threads = vec![];

        for t in 0..nthreads {
            let o  = nitems/nthreads * t;
            let bh = h.clone();
            let t = thread::spawn(move || {
                for i in 0..nitems { 
                    let mut m = bh.lock().unwrap();
                    let j = (i+o) % nitems;
                    // print!("t");
                    let b : usize = *(m.get(j).unwrap()).clone();
                    //print!("thread: {}, item: {}, val: {} \n", t,  j , b);
                    m[j] = Box::new(b+1);
                }
            });
            threads.push(t);
            for k in 0..4 {
                let bh = h.clone();
                let t = thread::spawn(move || {
                    for i in 0..3 {
                        for i in 0..nitems { 
                            let mut m = bh.lock().unwrap();
                            let b : usize = *(m.get(i).unwrap()).clone();
                        };
                    }
                });
                threads.push(t);
            }
        };
        for t in threads {
            let _ = t.join();
        };
        let mut v = h.lock().unwrap();
        { 
            for i in 0..nitems {
                assert!(*v[i] == nthreads);
            };
        }
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
            let o = txn.read(addr).unwrap();
        }
        {
            let mut txn = s.transaction();
            let old = txn.read(addr).unwrap();
            let mut new = old.copy();
            new.push_str(" mutated");
            txn.update(old, new);
            txn.apply();
        }
        {
            let mut txn = s.transaction();
            let o = txn.read(addr).unwrap();
        }
        {
            let mut txn = s.transaction();
            txn.delete(addr);
            txn.apply();
        }
    }
}
