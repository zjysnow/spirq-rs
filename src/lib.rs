//! # SPIR-Q: Light Weight SPIR-V Query Utility for Graphics.
//!
//! SPIR-Q is a light weight library for SPIR-V pipeline metadata query, which
//! can be very useful for dynamic graphics/compute pipeline construction,
//! shader debugging and so on. SPIR-Q is currently compatible with a subset of
//! SPIR-V 1.5, with most of graphics capabilities but no OpenCL kernel
//! capabilities covered.
//!
//! ## How-to
//!
//! ```ignore
//! // Load SPIR-V data into `[u32]` buffer `spv_words`.
//! let spv: SpirvBinary = spv_words.into();
//! let entries = spv.reflect().unwrap();
//! // All extracted entry point data are available in `entries`.
//! ```
//!
//! By calling [`reflect`] of the wrapper type [`SpirvBinary`], every entry
//! point in the binary are analyzed and reported as one or more
//! [`EntryPoint`]s. Each entry point has a [`Manifest`] that supports queries
//! from allocation requirement to fine-grained typing details.
//!
//! ## Size calculation
//!
//! The struct member offsets and array/matrix strides are specified in SPIR-V
//! files. With these information SPIR-Q deduce the minimal size required for
//! to contain an instance of a type. However, SPIR-Q cannot handle dynamically-
//! sized arrays, and it will treat such arrays as zero-sized. The user has to
//! handle such SSBO-like themselves via [`Type`] APIs.
//!
//! ## Symbol resolution
//!
//! SPIR-Q uses a very simple solution to help you locate any metadata including
//! input/output variables, descriptors and variables defined inside those
//! descriptors. We call it a [`Symbol`]. A symbol is a dot-separated list of
//! identifiers. Identifiers can be an index or a name literal (or empty for the
//! push constant block.)
//!
//! Input/output variables are referred to by their locations. The following
//! are examples of input/output variable symbols:
//!
//! ```ignore
//! 1
//! aTexCoord
//! vWorldPosition
//! 1.2 // ERROR: I/O variables cannot be nested.
//! gl_Position // WARNING: Built-in variables are ignored during reflection.
//! ```
//!
//! Descriptors have to be referred to with both the descriptor set number and
//! its binding point number specified. The following are valid symbols for
//! descriptor variables:
//!
//! ```ignore
//! 0.1 // Refering to the descriptor at set 0 on binding 1.
//! light.0 // Refering to the first member of block 'light'.
//! 1.0.bones.4 // Refering to the 5th element of array member `bones` in descriptor `1.0`.
//! .modelview // Push constants are referred to by an empty identifier.
//! ```
//!
//! Note: It should be noted that descriptor multibinds are treated like single-
//! binds because although they use the same syntax as arrays, they are not
//! actually arrays.
//!
//! Note: Although `spv` files generated directly from compilers normally keep
//! the nameing data, it should be noticed that names are debug information that
//! might be wiped out during compression.
//!
//! [`SpirvBinary`]: struct.SpirvBinary.html
//! [`EntryPoint`]: struct.EntryPoint.html
//! [`reflect`]: struct.SpirvBinary.html#method.reflect
//! [`Manifest`]: struct.Manifest.html
//! [`Type`]: ty/enum.Type.html
//! [`Symbol`]: sym/struct.Symbol.html
mod consts;
mod parse;
mod instr;
mod reflect;
pub mod sym;
pub mod error;
pub mod ty;

use std::convert::TryInto;
use std::collections::{HashMap};
use std::fmt;
use std::iter::FromIterator;
use std::ops::Deref;
use parse::{Instrs, Instr};
use ty::{Type, DescriptorType};
pub use sym::*;
pub use error::*;
pub use spirv_headers::ExecutionModel;
use num_derive::FromPrimitive;

/// SPIR-V program binary.
#[derive(Debug, Default, Clone)]
pub struct SpirvBinary(Vec<u32>);
impl From<Vec<u32>> for SpirvBinary {
    fn from(x: Vec<u32>) -> Self { SpirvBinary(x) }
}
impl FromIterator<u32> for SpirvBinary {
    fn from_iter<I: IntoIterator<Item=u32>>(iter: I) -> Self { SpirvBinary(iter.into_iter().collect::<Vec<u32>>()) }
}
impl From<Vec<u8>> for SpirvBinary {
    fn from(x: Vec<u8>) -> Self {
        if x.len() == 0 { return SpirvBinary::default(); }
        x.chunks_exact(4)
            .map(|x| x.try_into().unwrap())
            .map(match x[0] {
                0x03 => u32::from_le_bytes,
                0x07 => u32::from_be_bytes,
                _ => return SpirvBinary::default(),
            })
            .collect::<SpirvBinary>()
    }
}

impl SpirvBinary {
    pub(crate) fn instrs<'a>(&'a self) -> Instrs<'a> { Instrs::new(&self.0) }
    pub fn reflect(&self) -> Result<Box<[EntryPoint]>> {
        reflect::reflect_spirv(&self)
    }
    pub fn words(&self) -> &[u32] {
        &self.0
    }
    pub fn bytes(&self) -> &[u8] {
        unsafe {
            let len = self.0.len() * std::mem::size_of::<u32>();
            let ptr = self.0.as_ptr() as *const u8;
            std::slice::from_raw_parts(ptr, len)
        }
    }
    pub fn into_words(self) -> Vec<u32> { self.0 }
}


/// Internal hasher for type equality check.
pub(crate) fn hash<H: std::hash::Hash>(h: &H) -> u64 {
    use std::hash::Hasher;
    use std::collections::hash_map::DefaultHasher;
    let mut hasher = DefaultHasher::new();
    h.hash(&mut hasher);
    hasher.finish()
}


// Resource locationing.

/// Interface variable location and component.
#[derive(PartialEq, Eq, Hash, Default, Clone, Copy)]
pub struct InterfaceLocation(u32, u32);
impl InterfaceLocation {
    pub fn new(loc: u32, comp: u32) -> Self { InterfaceLocation(loc, comp) }

    pub fn loc(&self) -> u32 { self.0 }
    pub fn bind(&self) -> u32 { self.1 }
    pub fn into_inner(self) -> (u32, u32) { (self.0, self.1) }
}
impl fmt::Display for InterfaceLocation {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "(loc={}, comp={})", self.0, self.1)
    }
}
impl fmt::Debug for InterfaceLocation {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { (self as &dyn fmt::Display).fmt(f) }
}

/// Descriptor set and binding point carrier.
#[derive(PartialEq, Eq, Hash, Default, Clone, Copy)]
pub struct DescriptorBinding(u32, u32);
impl DescriptorBinding {
    pub fn new(desc_set: u32, bind_point: u32) -> Self { DescriptorBinding(desc_set, bind_point) }

    pub fn set(&self) -> u32 { self.0 }
    pub fn bind(&self) -> u32 { self.1 }
    pub fn into_inner(self) -> (u32, u32) { (self.0, self.1) }
}
impl fmt::Display for DescriptorBinding {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(f, "(set={}, bind={})", self.0, self.1)
    }
}
impl fmt::Debug for DescriptorBinding {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { (self as &dyn fmt::Display).fmt(f) }
}

#[derive(PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum ResourceLocator {
    Input(InterfaceLocation),
    Output(InterfaceLocation),
    Descriptor(DescriptorBinding),
}

// Resolution results.


/// Interface variables resolution result.
#[derive(Debug)]
pub struct InterfaceVariableResolution<'a> {
    /// Location of the current interface variable. It should be noted that
    /// matrix types can take more than one location.
    pub location: InterfaceLocation,
    /// Type of the resolution target.
    pub ty: &'a Type,
}

/// Push constant resolution result.
#[derive(Debug)]
pub struct PushConstantResolution<'a> {
    /// Type of the push constant block. This is expected to be struct.
    pub ty: &'a Type,
    /// Resolution of a variable in the push constant block, if the resolution
    /// doesn't end at the block.
    pub member_var_res: Option<MemberVariableResolution<'a>>,
}
/// Descriptor variable resolution result.
#[derive(Debug)]
pub struct DescriptorResolution<'a> {
    /// Descriptor set and binding point of the descriptor.
    pub desc_bind: DescriptorBinding,
    /// Type of the descriptor.
    pub desc_ty: &'a DescriptorType,
    /// Resolution of a variable in the descriptor, if the resolution doesn't
    /// end at a descriptor type.
    pub member_var_res: Option<MemberVariableResolution<'a>>,
}
/// Member variable resolution result.
#[derive(Debug)]
pub struct MemberVariableResolution<'a> {
    /// Offset to the resolution target from the beginning of buffer.
    pub offset: usize,
    /// Type of the resolution target.
    pub ty: &'a Type,
}

/// Access type of a variable.
#[repr(u32)]
#[derive(Debug, FromPrimitive, Clone, Copy, PartialEq, Eq)]
pub enum AccessType {
    /// The variable has only been read from.
    ReadOnly = 1,
    /// The variable has only been written to.
    WriteOnly = 2,
    /// The variable has been read from and written to.
    ReadWrite = 3,
}

/// A set of information used to describe variable typing and routing.
#[derive(Default, Clone)]
pub struct Manifest {
    pub(crate) push_const_ty: Option<Type>,
    pub(crate) input_map: HashMap<InterfaceLocation, Type>,
    pub(crate) output_map: HashMap<InterfaceLocation, Type>,
    pub(crate) desc_map: HashMap<DescriptorBinding, DescriptorType>,
    pub(crate) var_name_map: HashMap<String, ResourceLocator>,
    pub(crate) desc_access_map: HashMap<DescriptorBinding, AccessType>
}
impl Manifest {
    fn merge_push_const(&mut self, other: &Manifest) -> Result<()> {
        if let Some(Type::Struct(dst_struct_ty)) = self.push_const_ty.as_mut() {
            // Merge push constants scattered in different stages. This match
            // must success.
            if let Some(Type::Struct(src_struct_ty)) = other.push_const_ty.as_ref() {
                dst_struct_ty.merge(&src_struct_ty)?;
            }
            // It's guaranteed to be interface uniform so we don't have to check
            // the hash.
        } else {
            self.push_const_ty = other.push_const_ty.clone();
        }
        Ok(())
    }
    fn merge_descs(&mut self, other: &Manifest) -> Result<()> {
        use std::collections::hash_map::Entry::{Vacant, Occupied};
        for (desc_bind, desc_ty) in other.desc_map.iter() {
            match self.desc_map.entry(*desc_bind) {
                Vacant(entry) => { entry.insert(desc_ty.clone()); },
                Occupied(entry) => {
                    // Just regular descriptor types. Simply match the hashes.
                    if hash(entry.get()) != hash(&desc_ty) {
                        return Err(Error::MismatchedManifest);
                    }
                }
            }
        }
        Ok(())
    }
    fn merge_names(&mut self, other: &Manifest) -> Result<()> {
        use std::collections::hash_map::Entry::{Vacant, Occupied};
        for (name, locator) in other.var_name_map.iter() {
            match self.var_name_map.entry(name.to_owned()) {
                Vacant(entry) => { entry.insert(locator.clone()); },
                Occupied(entry) => if entry.get() != locator {
                    // Mismatched names are not allowed.
                    return Err(Error::MismatchedManifest);
                },
            }
        }
        Ok(())
    }
    fn merge_accesses(&mut self, other: &Manifest) -> Result<()> {
        for (desc_bind, access) in other.desc_access_map.iter() {
            if let Some(acc) = self.desc_access_map.get_mut(&desc_bind) {
                use num_traits::FromPrimitive;
                let access = *acc as u32 | *access as u32;
                *acc = AccessType::from_u32(access).unwrap();
            } else {
                self.desc_access_map.insert(*desc_bind, *access);
            }
        }
        Ok(())
    }
    /// Merge metadata records in another manifest into the current one IN
    /// ORDER. Inputs of the current manifest will kept; outputs will be
    /// replaced by the `other`'s; and descriptors will be aggregated to contain
    /// both set of metadata.
    pub fn merge(&mut self, other: &Manifest) -> Result<()> {
        self.output_map = other.output_map.clone();
        self.merge_push_const(other)?;
        self.merge_descs(other)?;
        self.merge_names(other)?;
        self.merge_accesses(other)?;
        Ok(())
    }
    /// Get the push constant type.
    pub fn get_push_const<'a>(&'a self) -> Option<&'a Type> {
        self.push_const_ty.as_ref()
    }
    /// Get the input interface variable type.
    pub fn get_input<'a>(&'a self, location: InterfaceLocation) -> Option<&'a Type> {
        self.input_map.get(&location)
    }
    /// Get the output interface variable type.
    pub fn get_output<'a>(&'a self, location: InterfaceLocation) -> Option<&'a Type> {
        self.output_map.get(&location)
    }
    /// Get the descriptor type at the given descriptor binding point.
    pub fn get_desc<'a>(&'a self, desc_bind: DescriptorBinding) -> Option<&'a DescriptorType> {
        self.desc_map.get(&desc_bind)
    }
    /// Get the name that also refers to the input at the given location.
    pub fn get_input_name<'a>(&'a self, location: InterfaceLocation) -> Option<&'a str> {
        self.var_name_map.iter()
            .find_map(|x| if let ResourceLocator::Input(loc) = x.1 {
                if *loc == location { Some(x.0.as_ref()) } else { None }
            } else { None })
    }
    /// Get the name that also refers to the output at the given location.
    pub fn get_output_name<'a>(&'a self, location: InterfaceLocation) -> Option<&'a str> {
        self.var_name_map.iter()
            .find_map(|x| if let ResourceLocator::Output(loc) = x.1 {
                if *loc == location { Some(x.0.as_ref()) } else { None }
            } else { None })
    }
    /// Get the name that also refers to the descriptor at the given descriptor
    /// binding.
    pub fn get_desc_name<'a>(&'a self, desc_bind: DescriptorBinding) -> Option<&'a str> {
        self.var_name_map.iter()
            .find_map(|x| if let ResourceLocator::Descriptor(db) = x.1 {
                if *db == desc_bind { Some(x.0.as_ref()) } else { None }
            } else { None })
    }
    /// Get the access pattern of the descriptor at the given descriptor
    /// binding.
    pub fn get_desc_access(&self, desc_bind: DescriptorBinding) -> Option<AccessType> {
        self.desc_access_map
            .get(&desc_bind)
            .map(|x| *x)
    }
    fn resolve_ivar<'a>(&self, map: &'a HashMap<InterfaceLocation, Type>, sym: &Sym) -> Option<InterfaceVariableResolution<'a>> {
        let mut segs = sym.segs();
        let location = match segs.next() {
            Some(Seg::Index(loc)) => {
                if let Some(Seg::Index(comp)) = segs.next() {
                    InterfaceLocation::new(loc as u32, comp as u32)
                } else { return None; }
            },
            Some(Seg::Name(name)) => {
                if let Some(ResourceLocator::Input(location)) = self.var_name_map.get(name) {
                    *location
                } else { return None }
            },
            _ => return None,
        };
        if segs.next().is_some() { return None }
        let ty = map.get(&location)?;
        let ivar_res = InterfaceVariableResolution { location, ty };
        Some(ivar_res)
    }
    /// Get the metadata of a input variable identified by a symbol.
    pub fn resolve_input<S: AsRef<Sym>>(&self, sym: S) -> Option<InterfaceVariableResolution> {
        self.resolve_ivar(&self.output_map, sym.as_ref())
    }
    /// Get the metadata of a output variable identified by a symbol.
    pub fn resolve_output<S: AsRef<Sym>>(&self, sym: S) -> Option<InterfaceVariableResolution> {
        self.resolve_ivar(&self.input_map, sym.as_ref())
    }
    /// Get the metadata of a descriptor variable identified by a symbol.
    /// If the exact variable cannot be resolved, the descriptor part of the
    /// resolution will still be returned, if possible.
    pub fn resolve_desc<S: AsRef<Sym>>(&self, sym: S) -> Option<DescriptorResolution> {
        let mut segs = sym.as_ref().segs();
        let desc_bind = match segs.next() {
            Some(Seg::Index(desc_set)) => {
                if let Some(Seg::Index(bind_point)) = segs.next() {
                    DescriptorBinding::new(desc_set as u32, bind_point as u32)
                } else { return None; }
            },
            Some(Seg::Name(name)) => {
                if let Some(ResourceLocator::Descriptor(desc_bind)) = self.var_name_map.get(name) {
                    *desc_bind
                } else { return None; }
            },
            _ => return None,
        };
        let desc_ty = self.desc_map.get(&desc_bind)?;
        let rem_sym = segs.remaining();
        let member_var_res = desc_ty.resolve(rem_sym);
        let desc_res = DescriptorResolution { desc_bind, desc_ty, member_var_res };
        Some(desc_res)
    }
    /// Get the metadata of a descriptor variable identified by a symbol.If the
    /// exact variable cannot be resolved, the descriptor part of the resolution
    /// will still be returned, if possible.
    pub fn resolve_push_const<S: AsRef<Sym>>(&self, sym: S) -> Option<PushConstantResolution> {
        let mut segs = sym.as_ref().segs();
        match segs.next() {
            Some(Seg::Empty) => {
                // Symbols started with an empty head, like ".modelView", is
                // used to identify push constants.
            },
            _ => return None,
        };
        let ty = self.push_const_ty.as_ref()?;
        let rem_sym = segs.remaining();
        let member_var_res = ty.resolve(rem_sym);
        let push_const_res = PushConstantResolution { ty, member_var_res };
        Some(push_const_res)
    }
    /// List all input locations
    pub fn inputs<'a>(&'a self) -> impl Iterator<Item=InterfaceVariableResolution<'a>> {
        self.input_map.iter()
            .map(|(&location, ty)| {
                InterfaceVariableResolution { location, ty }
            })
    }
    /// List all output locations in this manifest.
    pub fn outputs<'a>(&'a self) -> impl Iterator<Item=InterfaceVariableResolution<'a>> {
        self.output_map.iter()
            .map(|(&location, ty)|  {
                InterfaceVariableResolution { location, ty }
            })
    }
    /// List all descriptors in this manifest. Results will not contain anything
    /// about exact variables in buffers.
    pub fn descs<'a>(&'a self) -> impl Iterator<Item=DescriptorResolution<'a>> {
        self.desc_map.iter()
            .map(|(&desc_bind, desc_ty)| {
                DescriptorResolution{ desc_bind, desc_ty, member_var_res: None }
            })
    }
}


// SPIR-V program entry points.

/// Representing an entry point described in a SPIR-V.
#[derive(Clone)]
pub struct EntryPoint {
    /// Entry point execution model.
    pub exec_model: ExecutionModel,
    /// Name of the entry point.
    pub name: String,
    /// Manifest object that contains input, output and descriptor type
    /// information.
    pub manifest: Manifest,
}
impl Deref for EntryPoint {
    type Target = Manifest;
    fn deref(&self) -> &Self::Target { &self.manifest }
}
impl fmt::Debug for EntryPoint {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct(&self.name)
            .field("push_const", &self.manifest.push_const_ty)
            .field("inputs", &self.manifest.input_map)
            .field("outputs", &self.manifest.output_map)
            .field("descriptors", &self.manifest.desc_map)
            .finish()
    }
}
