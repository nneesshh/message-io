#![deny(missing_docs, warnings)]
//! Traits for unsafe downcasting from trait objects to & or &mut references of
//! concrete types. These should only be used if you are absolutely certain of the
//! type of the data in said trait object - there be dragons etc.
//!
//! Originally inspired by https://github.com/chris-morgan/anymap
//! and the implementation of `std::any::Any`.

use std::any::Any;
use std::mem;

/// A trait providing unchecked downcasting to its contents when stored
/// in a trait object.
pub trait UnsafeAny: Any {}

impl<T: Any> UnsafeAny for T {}

impl dyn UnsafeAny {
    /// Returns a reference to the contained value, assuming that it is of type `T`.
    ///
    /// # Safety
    ///
    /// If you are not _absolutely certain_ of `T` you should _not_ call this!
    #[inline(always)]
    pub unsafe fn downcast_ref_unchecked<T: Any>(&self) -> &T {
        &*(destructure_traitobject::data(self) as *const T)
    }

    /// Returns a mutable reference to the contained value, assuming that it is of type `T`.
    ///
    /// # Safety
    ///
    /// If you are not _absolutely certain_ of `T` you should _not_ call this!
    #[inline(always)]
    pub unsafe fn downcast_mut_unchecked<T: Any>(&mut self) -> &mut T {
        &mut *(destructure_traitobject::data_mut(self) as *mut T)
    }

    /// Returns a the contained value, assuming that it is of type `T`.
    ///
    /// # Safety
    ///
    /// If you are not _absolutely certain_ of `T` you should _not_ call this!
    #[inline(always)]
    pub unsafe fn downcast_unchecked<T: Any>(self: Box<dyn UnsafeAny>) -> Box<T> {
        let raw: *mut dyn UnsafeAny = mem::transmute(self);
        mem::transmute(destructure_traitobject::data_mut(raw))
    }
}

/// An extension trait for unchecked downcasting of trait objects.
/// # Safety
///
/// See specific funtion calls for areas to be aware of.
pub unsafe trait UnsafeAnyExt {
    /// Returns a reference to the contained value, assuming that it is of type `T`.
    ///
    /// # Safety
    ///
    /// If you are not _absolutely certain_ of `T` you should _not_ call this!
    #[inline(always)]
    unsafe fn downcast_ref_unchecked<T: Any>(&self) -> &T {
        &*(destructure_traitobject::data(self) as *const T)
    }

    /// Returns a mutable reference to the contained value, assuming that it is of type `T`.
    ///
    /// # Safety
    ///
    /// If you are not _absolutely certain_ of `T` you should _not_ call this!
    #[inline(always)]
    unsafe fn downcast_mut_unchecked<T: Any>(&mut self) -> &mut T {
        &mut *(destructure_traitobject::data_mut(self) as *mut T)
    }

    /// Returns a the contained value, assuming that it is of type `T`.
    ///
    /// # Safety
    ///
    /// If you are not _absolutely certain_ of `T` you should _not_ call this!
    #[inline(always)]
    unsafe fn downcast_unchecked<T: Any>(self: Box<Self>) -> Box<T> {
        let raw: *mut Self = mem::transmute(self);
        mem::transmute(destructure_traitobject::data_mut(raw))
    }
}

mod destructure_traitobject {
    /// Get the data pointer from this trait object.
    ///
    /// Highly unsafe, as there is no information about the type of the data.
    #[inline(always)]
    pub unsafe fn data<T: ?Sized>(val: *const T) -> *const () {
        val as *const ()
    }

    /// Get the data pointer from this trait object, mutably.
    ///
    /// Highly unsafe, as there is no information about the type of the data.
    #[inline(always)]
    pub unsafe fn data_mut<T: ?Sized>(val: *mut T) -> *mut () {
        val as *mut ()
    }
}

unsafe impl UnsafeAnyExt for dyn Any {}
unsafe impl UnsafeAnyExt for dyn UnsafeAny {}
unsafe impl UnsafeAnyExt for dyn Any + Send {}
unsafe impl UnsafeAnyExt for dyn Any + Sync {}
unsafe impl UnsafeAnyExt for dyn Any + Send + Sync {}
unsafe impl UnsafeAnyExt for dyn UnsafeAny + Send {}
unsafe impl UnsafeAnyExt for dyn UnsafeAny + Sync {}
unsafe impl UnsafeAnyExt for dyn UnsafeAny + Send + Sync {}

#[cfg(test)]
mod test {
    use std::any::Any;

    use super::{destructure_traitobject, UnsafeAny, UnsafeAnyExt};

    #[allow(unstable_name_collisions)]
    #[test]
    fn test_simple() {
        let x = &7 as &dyn Send;
        unsafe { assert!(&7 == std::mem::transmute::<_, &i32>(destructure_traitobject::data(x))) };
    }

    #[allow(unstable_name_collisions)]
    #[test]
    fn test_simple_downcast_ext() {
        let a = Box::new(7usize) as Box<dyn Any>;
        unsafe {
            assert_eq!(*a.downcast_ref_unchecked::<usize>(), 7);
        }

        let mut a = Box::new(7usize) as Box<dyn Any>;
        unsafe {
            assert_eq!(*a.downcast_mut_unchecked::<usize>(), 7);
        }

        let mut a = Box::new(7usize) as Box<dyn Any>;
        unsafe {
            *a.downcast_mut_unchecked::<usize>() = 8;
            assert_eq!(*a.downcast_mut_unchecked::<usize>(), 8);
        }
    }

    #[allow(unstable_name_collisions)]
    #[test]
    fn test_simple_downcast_inherent() {
        let a = Box::new(7usize) as Box<dyn UnsafeAny>;
        unsafe {
            assert_eq!(*a.downcast_ref_unchecked::<usize>(), 7);
        }

        let mut a = Box::new(7usize) as Box<dyn UnsafeAny>;
        unsafe {
            assert_eq!(*a.downcast_mut_unchecked::<usize>(), 7);
        }

        let mut a = Box::new(7usize) as Box<dyn UnsafeAny>;
        unsafe {
            *a.downcast_mut_unchecked::<usize>() = 8;
            assert_eq!(*a.downcast_mut_unchecked::<usize>(), 8);
        }
    }

    #[allow(unstable_name_collisions)]
    #[test]
    fn test_box_downcast_no_double_free() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        struct Dropper {
            x: Arc<AtomicUsize>,
        }

        impl Drop for Dropper {
            fn drop(&mut self) {
                self.x.fetch_add(1, Ordering::SeqCst);
            }
        }

        let x = Arc::new(AtomicUsize::new(0));
        let a = Box::new(Dropper { x: x.clone() }) as Box<dyn UnsafeAny>;

        let dropper = unsafe { a.downcast_unchecked::<Dropper>() };
        drop(dropper);

        assert_eq!(x.load(Ordering::SeqCst), 1);

        // Test the UnsafeAnyExt implementation.
        let x = Arc::new(AtomicUsize::new(0));
        let a = Box::new(Dropper { x: x.clone() }) as Box<dyn Any>;

        let dropper = unsafe { a.downcast_unchecked::<Dropper>() };
        drop(dropper);

        assert_eq!(x.load(Ordering::SeqCst), 1);
    }
}
