use std::any::Any;
use std::cell::Cell;
use std::marker::PhantomData;
use std::rc::Rc;

use crate::{
    Chunk, FromLua, FromLuaMulti, Function, IntoLua, LuaResult, LuaState, Table, UdValue,
    UserDataTrait, Value,
};

use crate::lua_api::Lua;

fn typed_scope_arg<T: FromLua>(state: &mut LuaState, index: usize) -> LuaResult<T> {
    let value = state.get_arg(index).unwrap_or_default();
    T::from_lua(value, state).map_err(|msg| state.error(msg))
}

#[doc(hidden)]
pub trait ScopedLuaCallback<Args, R> {
    fn invoke_typed(&self, state: &mut LuaState) -> LuaResult<usize>;
}

#[doc(hidden)]
pub trait ScopedLuaCallbackWith<Data, Args, R> {
    fn invoke_typed_with(&self, data: &Data, state: &mut LuaState) -> LuaResult<usize>;
}

#[doc(hidden)]
pub trait ScopedLuaCallbackMutWith<Data, Args, R> {
    fn invoke_typed_with_mut(&self, data: &mut Data, state: &mut LuaState) -> LuaResult<usize>;
}

struct CallbackResource<'scope> {
    ptr: *mut (),
    drop_fn: unsafe fn(*mut ()),
    _marker: PhantomData<&'scope ()>,
}

impl<'scope> CallbackResource<'scope> {
    fn new<T: 'scope>(callback: Box<T>) -> Self {
        unsafe fn drop_box<T>(ptr: *mut ()) {
            drop(unsafe { Box::from_raw(ptr.cast::<T>()) });
        }

        CallbackResource {
            ptr: Box::into_raw(callback).cast::<()>(),
            drop_fn: drop_box::<T>,
            _marker: PhantomData,
        }
    }
}

impl Drop for CallbackResource<'_> {
    fn drop(&mut self) {
        unsafe { (self.drop_fn)(self.ptr) };
    }
}

impl<Func, R> ScopedLuaCallback<(), R> for Func
where
    Func: Fn() -> R,
    R: IntoLua,
{
    fn invoke_typed(&self, state: &mut LuaState) -> LuaResult<usize> {
        match (self)().into_lua(state) {
            Ok(count) => Ok(count),
            Err(msg) => Err(state.error(msg)),
        }
    }
}

impl<Data, Func, R> ScopedLuaCallbackWith<Data, (), R> for Func
where
    Func: Fn(&Data) -> R,
    R: IntoLua,
{
    fn invoke_typed_with(&self, data: &Data, state: &mut LuaState) -> LuaResult<usize> {
        match (self)(data).into_lua(state) {
            Ok(count) => Ok(count),
            Err(msg) => Err(state.error(msg)),
        }
    }
}

impl<Data, Func, R> ScopedLuaCallbackMutWith<Data, (), R> for Func
where
    Func: Fn(&mut Data) -> R,
    R: IntoLua,
{
    fn invoke_typed_with_mut(&self, data: &mut Data, state: &mut LuaState) -> LuaResult<usize> {
        match (self)(data).into_lua(state) {
            Ok(count) => Ok(count),
            Err(msg) => Err(state.error(msg)),
        }
    }
}

macro_rules! impl_scoped_lua_callback {
    ($(($(($ty:ident, $value:ident) => $index:literal),+)),* $(,)?) => {
        $(
            impl<Func, R, $($ty),+> ScopedLuaCallback<($($ty,)+), R> for Func
            where
                Func: Fn($($ty),+) -> R,
                R: IntoLua,
                $($ty: FromLua),+
            {
                fn invoke_typed(&self, state: &mut LuaState) -> LuaResult<usize> {
                    $(
                        let $value = typed_scope_arg::<$ty>(state, $index)?;
                    )+

                    match (self)($($value),+).into_lua(state) {
                        Ok(count) => Ok(count),
                        Err(msg) => Err(state.error(msg)),
                    }
                }
            }
        )*
    };
}

impl_scoped_lua_callback!(
    ((A, a) => 1),
    ((A, a) => 1, (B, b) => 2),
    ((A, a) => 1, (B, b) => 2, (C, c) => 3),
    ((A, a) => 1, (B, b) => 2, (C, c) => 3, (D, d) => 4),
    ((A, a) => 1, (B, b) => 2, (C, c) => 3, (D, d) => 4, (E, e) => 5),
    ((A, a) => 1, (B, b) => 2, (C, c) => 3, (D, d) => 4, (E, e) => 5, (T6, t6) => 6),
    ((A, a) => 1, (B, b) => 2, (C, c) => 3, (D, d) => 4, (E, e) => 5, (T6, t6) => 6, (T7, t7) => 7),
    ((A, a) => 1, (B, b) => 2, (C, c) => 3, (D, d) => 4, (E, e) => 5, (T6, t6) => 6, (T7, t7) => 7, (T8, t8) => 8)
);

macro_rules! impl_scoped_lua_callback_with {
    ($(($(($ty:ident, $value:ident) => $index:literal),+)),* $(,)?) => {
        $(
            impl<Data, Func, R, $($ty),+> ScopedLuaCallbackWith<Data, ($($ty,)+), R> for Func
            where
                Func: Fn(&Data, $($ty),+) -> R,
                R: IntoLua,
                $($ty: FromLua),+
            {
                fn invoke_typed_with(&self, data: &Data, state: &mut LuaState) -> LuaResult<usize> {
                    $(
                        let $value = typed_scope_arg::<$ty>(state, $index)?;
                    )+

                    match (self)(data, $($value),+).into_lua(state) {
                        Ok(count) => Ok(count),
                        Err(msg) => Err(state.error(msg)),
                    }
                }
            }
        )*
    };
}

impl_scoped_lua_callback_with!(
    ((A, a) => 1),
    ((A, a) => 1, (B, b) => 2),
    ((A, a) => 1, (B, b) => 2, (C, c) => 3),
    ((A, a) => 1, (B, b) => 2, (C, c) => 3, (D, d) => 4),
    ((A, a) => 1, (B, b) => 2, (C, c) => 3, (D, d) => 4, (E, e) => 5),
    ((A, a) => 1, (B, b) => 2, (C, c) => 3, (D, d) => 4, (E, e) => 5, (T6, t6) => 6),
    ((A, a) => 1, (B, b) => 2, (C, c) => 3, (D, d) => 4, (E, e) => 5, (T6, t6) => 6, (T7, t7) => 7),
    ((A, a) => 1, (B, b) => 2, (C, c) => 3, (D, d) => 4, (E, e) => 5, (T6, t6) => 6, (T7, t7) => 7, (T8, t8) => 8)
);

macro_rules! impl_scoped_lua_callback_mut_with {
    ($(($(($ty:ident, $value:ident) => $index:literal),+)),* $(,)?) => {
        $(
            impl<Data, Func, R, $($ty),+> ScopedLuaCallbackMutWith<Data, ($($ty,)+), R> for Func
            where
                Func: Fn(&mut Data, $($ty),+) -> R,
                R: IntoLua,
                $($ty: FromLua),+
            {
                fn invoke_typed_with_mut(&self, data: &mut Data, state: &mut LuaState) -> LuaResult<usize> {
                    $(
                        let $value = typed_scope_arg::<$ty>(state, $index)?;
                    )+

                    match (self)(data, $($value),+).into_lua(state) {
                        Ok(count) => Ok(count),
                        Err(msg) => Err(state.error(msg)),
                    }
                }
            }
        )*
    };
}

impl_scoped_lua_callback_mut_with!(
    ((A, a) => 1),
    ((A, a) => 1, (B, b) => 2),
    ((A, a) => 1, (B, b) => 2, (C, c) => 3),
    ((A, a) => 1, (B, b) => 2, (C, c) => 3, (D, d) => 4),
    ((A, a) => 1, (B, b) => 2, (C, c) => 3, (D, d) => 4, (E, e) => 5),
    ((A, a) => 1, (B, b) => 2, (C, c) => 3, (D, d) => 4, (E, e) => 5, (T6, t6) => 6),
    ((A, a) => 1, (B, b) => 2, (C, c) => 3, (D, d) => 4, (E, e) => 5, (T6, t6) => 6, (T7, t7) => 7),
    ((A, a) => 1, (B, b) => 2, (C, c) => 3, (D, d) => 4, (E, e) => 5, (T6, t6) => 6, (T7, t7) => 7, (T8, t8) => 8)
);

fn scoped_expired_error() -> &'static str {
    "scoped value is no longer available"
}

struct ScopedBorrowedUserData<T: UserDataTrait> {
    ptr: *mut T,
    active: Rc<Cell<bool>>,
}

impl<T: UserDataTrait> ScopedBorrowedUserData<T> {
    fn new(reference: &mut T, active: Rc<Cell<bool>>) -> Self {
        ScopedBorrowedUserData {
            ptr: reference as *mut T,
            active,
        }
    }

    fn alive(&self) -> bool {
        self.active.get()
    }
}

impl<T: UserDataTrait> UserDataTrait for ScopedBorrowedUserData<T> {
    fn type_name(&self) -> &'static str {
        if self.alive() {
            unsafe { &*self.ptr }.type_name()
        } else {
            "scoped_userdata"
        }
    }

    fn get_field(&self, key: &str) -> Option<UdValue> {
        if !self.alive() {
            return None;
        }
        unsafe { &*self.ptr }.get_field(key)
    }

    fn set_field(&mut self, key: &str, value: UdValue) -> Option<Result<(), String>> {
        if !self.alive() {
            return Some(Err(scoped_expired_error().to_owned()));
        }
        unsafe { &mut *self.ptr }.set_field(key, value)
    }

    fn lua_tostring(&self) -> Option<String> {
        if !self.alive() {
            return Some(scoped_expired_error().to_owned());
        }
        unsafe { &*self.ptr }.lua_tostring()
    }

    fn lua_eq(&self, other: &dyn UserDataTrait) -> Option<bool> {
        if !self.alive() {
            return Some(false);
        }
        unsafe { &*self.ptr }.lua_eq(other)
    }

    fn lua_lt(&self, other: &dyn UserDataTrait) -> Option<bool> {
        if !self.alive() {
            return None;
        }
        unsafe { &*self.ptr }.lua_lt(other)
    }

    fn lua_le(&self, other: &dyn UserDataTrait) -> Option<bool> {
        if !self.alive() {
            return None;
        }
        unsafe { &*self.ptr }.lua_le(other)
    }

    fn lua_len(&self) -> Option<UdValue> {
        if !self.alive() {
            return None;
        }
        unsafe { &*self.ptr }.lua_len()
    }

    fn lua_unm(&self) -> Option<UdValue> {
        if !self.alive() {
            return None;
        }
        unsafe { &*self.ptr }.lua_unm()
    }

    fn lua_add(&self, other: &UdValue) -> Option<UdValue> {
        if !self.alive() {
            return None;
        }
        unsafe { &*self.ptr }.lua_add(other)
    }

    fn lua_sub(&self, other: &UdValue) -> Option<UdValue> {
        if !self.alive() {
            return None;
        }
        unsafe { &*self.ptr }.lua_sub(other)
    }

    fn lua_mul(&self, other: &UdValue) -> Option<UdValue> {
        if !self.alive() {
            return None;
        }
        unsafe { &*self.ptr }.lua_mul(other)
    }

    fn lua_div(&self, other: &UdValue) -> Option<UdValue> {
        if !self.alive() {
            return None;
        }
        unsafe { &*self.ptr }.lua_div(other)
    }

    fn lua_mod(&self, other: &UdValue) -> Option<UdValue> {
        if !self.alive() {
            return None;
        }
        unsafe { &*self.ptr }.lua_mod(other)
    }

    fn lua_concat(&self, other: &UdValue) -> Option<UdValue> {
        if !self.alive() {
            return None;
        }
        unsafe { &*self.ptr }.lua_concat(other)
    }

    fn lua_call(&self) -> Option<crate::CFunction> {
        if !self.alive() {
            return None;
        }
        unsafe { &*self.ptr }.lua_call()
    }

    fn lua_next(&self, control: &UdValue) -> Option<(UdValue, UdValue)> {
        if !self.alive() {
            return None;
        }
        unsafe { &*self.ptr }.lua_next(control)
    }

    fn field_names(&self) -> &'static [&'static str] {
        if !self.alive() {
            return &[];
        }
        unsafe { &*self.ptr }.field_names()
    }

    fn as_any(&self) -> &dyn Any {
        if self.alive() {
            unsafe { &*self.ptr }.as_any()
        } else {
            self
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        if self.alive() {
            unsafe { &mut *self.ptr }.as_any_mut()
        } else {
            self
        }
    }
}

/// Lexical scope for non-`'static` Lua values.
pub struct Scope<'scope, 'lua> {
    lua: &'scope mut Lua,
    active: Rc<Cell<bool>>,
    callbacks: Vec<CallbackResource<'scope>>,
    _lua: PhantomData<&'lua mut Lua>,
}

impl<'scope, 'lua> Scope<'scope, 'lua> {
    pub(crate) fn new(lua: &'scope mut Lua) -> Self {
        Scope {
            lua,
            active: Rc::new(Cell::new(true)),
            callbacks: Vec::new(),
            _lua: PhantomData,
        }
    }

    /// Access the underlying high-level Lua runtime within this scope.
    pub fn lua(&mut self) -> &mut Lua {
        self.lua
    }

    /// Return a handle to the global environment table.
    pub fn globals(&mut self) -> Table {
        self.lua.globals()
    }

    /// Return a chunk builder bound to this scope's Lua borrow.
    pub fn load<'a>(&'a mut self, source: &str) -> Chunk<'a> {
        self.lua.load(source)
    }

    /// Create a scoped Lua function from a `'static` Rust callback.
    pub fn create_function<F, Args, R>(
        &mut self,
        f: F,
    ) -> LuaResult<ScopedFunction<'scope, 'static>>
    where
        F: ScopedLuaCallback<Args, R> + 'static,
    {
        let callback = CallbackResource::new(Box::new(f));
        let callback_ptr = callback.ptr as usize;
        self.callbacks.push(callback);

        let active = self.active.clone();
        let function = self.lua.create_raw_function(move |state| {
            if !active.get() {
                return Err(state.error(scoped_expired_error().to_owned()));
            }

            let callback = unsafe { &*(callback_ptr as *const F) };
            callback.invoke_typed(state)
        })?;

        Ok(ScopedFunction::new(function))
    }

    /// Create a scoped Lua function that borrows Rust data from this lexical scope.
    pub fn create_function_with<'data, Data, F, Args, R>(
        &mut self,
        data: &'data Data,
        f: F,
    ) -> LuaResult<ScopedFunction<'scope, 'data>>
    where
        F: ScopedLuaCallbackWith<Data, Args, R> + 'static,
    {
        let callback = CallbackResource::new(Box::new(f));
        let callback_ptr = callback.ptr as usize;
        let data_ptr = (data as *const Data).cast::<()>() as usize;
        self.callbacks.push(callback);

        let active = self.active.clone();
        let function = self.lua.create_raw_function(move |state| {
            if !active.get() {
                return Err(state.error(scoped_expired_error().to_owned()));
            }

            let callback = unsafe { &*(callback_ptr as *const F) };
            let data = unsafe { &*(data_ptr as *const Data) };
            callback.invoke_typed_with(data, state)
        })?;

        Ok(ScopedFunction::new(function))
    }

    /// Create a scoped Lua function that mutably borrows Rust data from this lexical scope.
    pub fn create_function_mut_with<'data, Data, F, Args, R>(
        &mut self,
        data: &'data mut Data,
        f: F,
    ) -> LuaResult<ScopedFunction<'scope, 'data>>
    where
        F: ScopedLuaCallbackMutWith<Data, Args, R> + 'static,
    {
        let callback = CallbackResource::new(Box::new(f));
        let callback_ptr = callback.ptr as usize;
        let data_ptr = (data as *mut Data).cast::<()>() as usize;
        self.callbacks.push(callback);

        let active = self.active.clone();
        let function = self.lua.create_raw_function(move |state| {
            if !active.get() {
                return Err(state.error(scoped_expired_error().to_owned()));
            }

            let callback = unsafe { &*(callback_ptr as *const F) };
            let data = unsafe { &mut *(data_ptr as *mut Data) };
            callback.invoke_typed_with_mut(data, state)
        })?;

        Ok(ScopedFunction::new(function))
    }

    /// Create borrowed userdata tied to this lexical scope.
    pub fn create_userdata_ref<T: UserDataTrait>(
        &mut self,
        reference: &mut T,
    ) -> LuaResult<ScopedUserData<'scope, T>> {
        let ptr = reference as *mut T;
        let active = self.active.clone();
        let wrapper = ScopedBorrowedUserData::new(reference, active.clone());
        let userdata = self.lua.create_userdata_value(wrapper)?;
        Ok(ScopedUserData::new(userdata, ptr, active))
    }
}

impl Drop for Scope<'_, '_> {
    fn drop(&mut self) {
        self.active.set(false);
    }
}

/// A Lua function handle tied to a lexical scope.
#[derive(Debug)]
pub struct ScopedFunction<'scope, 'data> {
    inner: Function,
    _marker: PhantomData<(&'scope mut (), &'data ())>,
}

impl<'scope, 'data> ScopedFunction<'scope, 'data> {
    fn new(inner: Function) -> Self {
        ScopedFunction {
            inner,
            _marker: PhantomData,
        }
    }

    #[inline]
    pub fn call<A: IntoLua, R: FromLuaMulti>(&self, args: A) -> LuaResult<R> {
        self.inner.call(args)
    }

    #[inline]
    pub fn call1<A: IntoLua, R: FromLua>(&self, args: A) -> LuaResult<R> {
        self.inner.call1(args)
    }
}

impl IntoLua for ScopedFunction<'_, '_> {
    fn into_lua(self, state: &mut LuaState) -> Result<usize, String> {
        self.inner.into_lua(state)
    }
}

impl IntoLua for &ScopedFunction<'_, '_> {
    fn into_lua(self, state: &mut LuaState) -> Result<usize, String> {
        (&self.inner).into_lua(state)
    }
}

/// Borrowed userdata handle tied to a lexical scope.
#[derive(Debug)]
pub struct ScopedUserData<'scope, T: UserDataTrait> {
    inner: Value,
    ptr: *mut T,
    active: Rc<Cell<bool>>,
    _marker: PhantomData<&'scope mut T>,
}

impl<'scope, T: UserDataTrait> ScopedUserData<'scope, T> {
    fn new(inner: Value, ptr: *mut T, active: Rc<Cell<bool>>) -> Self {
        ScopedUserData {
            inner,
            ptr,
            active,
            _marker: PhantomData,
        }
    }

    pub fn get(&self) -> LuaResult<&T> {
        if !self.active.get() {
            return Err(crate::LuaError::RuntimeError);
        }
        Ok(unsafe { &*self.ptr })
    }

    pub fn get_mut(&mut self) -> LuaResult<&mut T> {
        if !self.active.get() {
            return Err(crate::LuaError::RuntimeError);
        }
        Ok(unsafe { &mut *self.ptr })
    }

    pub fn type_name(&self) -> LuaResult<&'static str> {
        self.get().map(UserDataTrait::type_name)
    }
}

impl<T: UserDataTrait> IntoLua for ScopedUserData<'_, T> {
    fn into_lua(self, state: &mut LuaState) -> Result<usize, String> {
        self.inner.into_lua(state)
    }
}

impl<T: UserDataTrait> IntoLua for &ScopedUserData<'_, T> {
    fn into_lua(self, state: &mut LuaState) -> Result<usize, String> {
        (&self.inner).into_lua(state)
    }
}
