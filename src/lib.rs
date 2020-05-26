#![feature(fn_traits, unboxed_closures, const_extern_fn)]

//! # Overloadf
//!
//! ** Let function overloading possible in rust **
//!
//! With a single attribute on top of the function, you can overload the function with different
//! parameters. Current implementation still has some flaws and todo items, so use at your own
//! risk.
//!
//! This library is based on some unstable features.
//! To use this library, please put the following lines in crate root and the beginning of test
//! files:
//! ```rust,no_run
//! #![feature(fn_traits, unboxed_closures)]
//! ```
//!
//! There are some features that cannot be achieved until now:
//! - unsafe function overloading
//! - const function overloading
//! - different privacy setting on function overloading (will pickup the privacy setting in first
//! function and apply to all)
//! - function overloading inside traits (for limited cases)
//!
//! ## Examples:
//! simple one:
//! ```rust
//! #![feature(fn_traits, unboxed_closures)]
//!
//! use overloadf::*;
//!
//! #[overload]
//! pub fn xdd() -> i32 {
//!     5_i32
//! }
//!
//! #[overload]
//! pub fn xdd(number: i32) -> i32 {
//!     number * 3
//! }
//!
//! #[overload]
//! pub fn xdd(number: u8) {
//!     println!("{}", number);
//! }
//!
//! #[overload]
//! pub unsafe fn xdd(number: &u64) -> u64 {
//!     let n = number as *const u64;
//!     *n * 4
//! }
//!
//! assert_eq!(xdd(3_i32), 9_i32);
//! let c: &u64 = &6_u64;
//! assert_eq!(xdd(c), 24_u64); // unsafe function is not supported.
//! assert_eq!(xdd(), 5_i32);
//! ```
//!
//! with generic and custom type:
//! ```rust
//! #![feature(fn_traits, unboxed_closures)]
//!
//! use overloadf::*;
//! use std::ops::MulAssign;
//! use std::fmt::Debug;
//! #[overload]
//! pub fn xdd<T: Copy + Debug + MulAssign<i32>>(mut number: T) -> T
//! where
//!     T: PartialEq,
//! {
//!     println!("number {:?}", number);
//!     number *= 3_i32;
//!     number
//! }
//!
//! struct ABC;
//! impl Debug for ABC {
//!     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//!         write!(f, "xdddd")
//!     }
//! }
//!
//! #[overload]
//! pub fn xdd(number: ABC) -> () {
//!     println!("number {:?}", number);
//! }
//! let aa = 123;
//! assert_eq!(xdd(aa), 369);
//! assert_eq!(xdd(ABC), ());
//! ```
//! async / await:
//! ```rust
//! #![feature(fn_traits, unboxed_closures)]
//! use overloadf::*;
//!
//! #[overload]
//! pub async fn xdd(number: i32) -> i32 {
//!     number + 3
//! }
//! #[overload]
//! pub async fn xdd(number: i64) -> i64 {
//!     number + 4
//! }
//! assert_eq!(futures::executor::block_on(xdd(3_i32)), 6);
//! assert_eq!(futures::executor::block_on(xdd(3_i64)), 7);
//! ```
//!
//! type conflict might happen if generic overlaps with the definition of implemented types:
//! ```rust,compile_fail
//! #![feature(fn_traits, unboxed_closures)]
//!
//! use overloadf::*;
//! use std::ops::Mul;
//! use std::fmt::Debug;
//! #[overload]
//! pub fn xdd(number: i32) -> i32 {
//!     number * 2
//! }
//! #[overload]
//! pub fn xdd<T: Copy + Debug + Mul<i32>>(number: T) -> T {
//!     number * 3_i32
//! }
//! ```
//!
//! for trait methods (notice that trait for overload must inherit Sized):
//! ```rust
//! #![feature(fn_traits, unboxed_closures)]
//! use overloadf::*;
//! #[overload]
//! trait Xdd: Sized {
//!     fn new(input: i32) -> Self;
//!     fn new(input :u32) -> Self;
//! }
//! struct Haha {
//!     a: u32,
//!     b: i32,
//! }
//! #[overload]
//! impl Xdd for Haha {
//!     fn new(b: i32) -> Self {
//!         Self {
//!             a: 1,
//!             b,
//!         }
//!     }
//!     fn new(a: u32) -> Self {
//!         Self {
//!             a,
//!             b: 2,
//!         }
//!     }
//! }
//! let haha = Haha::new(12_i32);
//! assert_eq!(haha.a, 1_u32);
//! assert_eq!(haha.b, 12_i32);
//! let haha = Haha::new(9_u32);
//! assert_eq!(haha.a, 9_u32);
//! assert_eq!(haha.b, 2_i32);
//! ```
//!
//! trait with generics:
//! ```rust
//! #![feature(fn_traits, unboxed_closures)]
//! use overloadf::*;
//! #[overload]
//! trait Xdd<T: Sized>: Sized {
//!     type J: Into<i32>;
//!     fn new(input: i32) -> T where T: Debug;
//!     // trait function with default implementation
//!     fn new(input: u32) -> String {
//!         "please use new(input1, input2) instead".to_string()
//!     }
//!     fn new<I: Into<u32>>(input1: I, input2: Self::J) -> T where T: Debug;
//! }
//! #[derive(Debug)]
//! struct Haha {
//!     a: u32,
//!     b: i32,
//! }
//! #[overload]
//! impl Xdd<Haha> for Haha {
//!     type J = i32;
//!     fn new(b: i32) -> Haha {
//!         Self {
//!             a: 1,
//!             b,
//!         }
//!     }
//!     fn new<I: Into<u32>>(a: I, b: Self::J) -> Haha {
//!         Self {
//!             a: a.into(),
//!             b,
//!         }
//!     }
//! }
//! let haha = Haha::new(12_i32);
//! assert_eq!(haha.a, 1_u32);
//! assert_eq!(haha.b, 12_i32);
//! let haha = Haha::new(9_u32, 2_i32);
//! assert_eq!(haha.a, 9_u32);
//! assert_eq!(haha.b, 2_i32);
//! assert_eq!(Haha::new(3_u32), "please use new(input1, input2) instead".to_string());
//! ```
//!
//! non-trait impl:
//! ```rust
//! #![feature(fn_traits, unboxed_closures)]
//! use overloadf::*;
//! #[derive(Debug)]
//! pub struct Haha {
//!     a: u32,
//!     b: i32,
//! }
//! #[overload]
//! impl Haha {
//!     pub fn new(b: i32) -> Self {
//!         Self {
//!             a: 1,
//!             b,
//!         }
//!     }
//!     pub fn new(a: u32) -> Self {
//!         Self {
//!             a,
//!             b: 2,
//!         }
//!     }
//!     // self in function parameter will be converted to associated type
//!     // that means no more syntax sugar like haha.normal()
//!     pub fn normal(&self) -> String {
//!         format!("{:?}", self)
//!     }
//!     pub fn normal(&self, prefix: &str) -> String {
//!         format!("{} {:?}", prefix, self)
//!     }
//!     // function without overloading is not influenced
//!     pub fn display(&self) -> String {
//!         format!("{:?}", self)
//!     }
//! }
//! let haha = Haha::new(12_i32);
//! assert_eq!(haha.a, 1_u32);
//! assert_eq!(haha.b, 12_i32);
//! let haha = Haha::new(9_u32);
//! assert_eq!(haha.a, 9_u32);
//! assert_eq!(haha.b, 2_i32);
//! assert_eq!(Haha::normal(&haha), "Haha { a: 9, b: 2 }");
//! assert_eq!(Haha::normal(&haha, "abc"), "abc Haha { a: 9, b: 2 }");
//! assert_eq!(haha.display(), "Haha { a: 9, b: 2 }");
//! ```
//!
//! default attribute:
//!
//! in functions(not yet for traits), now you could decorate parameters with default values.
//! the reason why we don't support `parameter = value` syntax directly is that in derived
//! `TokenStream`, values inside will go through compiler first for syntax check.
//! The default value syntax will be treated as a compile error and forbid us from parsing
//! and generating valid Tokens.
//!
//! example:
//! ```rust
//! #![feature(fn_traits, unboxed_closures)]
//! use overloadf::*;
//! #[overload]
//! fn xdd(#[default(= 5_i32)] a: i32, #[default(= 32_u64)] b: u64) -> u64 { b - (a as u64) }
//! assert_eq!(xdd(3_i32), 29_u64);
//! assert_eq!(xdd(4_i32, 4_u64), 0);
//! assert_eq!(xdd(), 27_u64);
//! // compile error: expect i32 found u64
//! // assert_eq!(xdd(6_u64), 1_u64);
//! ```
//!
//! ```rust
//! #![feature(fn_traits, unboxed_closures)]
//! use overloadf::*;
//! #[overload]
//! fn xdd(#[default(= 5_i32)] a: i32, b: u8, #[default(= 32_u64)] c: u64) -> u64 { c + (b as u64) - (a as u64) }
//! assert_eq!(xdd(4_i32, 7_u8), 35_u64);
//! assert_eq!(xdd(3_u8), 30_u64);
//! ```

pub extern crate overloadf_derive;
pub use overloadf_derive::overload;
