use std::borrow::Cow;
use std::rc::Rc;

use either::Either;
use inflections::Inflect;
use svd::{Access, EnumeratedValues, Field, Peripheral, Register, RegisterInfo,
          Usage};
use syn::{Ident, IntTy, Lit};

use errors::*;

/// List of chars that some vendors use in their peripheral/field names but
/// that are not valid in Rust ident
const BLACKLIST_CHARS: &'static [char] = &['(', ')'];

pub trait ToSanitizedPascalCase {
    fn to_sanitized_pascal_case(&self) -> Cow<str>;
}

pub trait ToSanitizedSnakeCase {
    fn to_sanitized_snake_case(&self) -> Cow<str>;
}

impl ToSanitizedSnakeCase for str {
    fn to_sanitized_snake_case(&self) -> Cow<str> {
        macro_rules! keywords {
            ($s:expr, $($kw:ident),+,) => {
                Cow::from(match &$s.to_lowercase()[..] {
                    $(stringify!($kw) => concat!(stringify!($kw), "_")),+,
                    _ => return Cow::from($s.to_snake_case())
                })
            }
        }

        let s = self.replace(BLACKLIST_CHARS, "");

        match s.chars().next().unwrap_or('\0') {
            '0' | '1' | '2' | '3' | '4' | '5' | '6' | '7' | '8' | '9' => {
                Cow::from(format!("_{}", s.to_snake_case()))
            }
            _ => {
                keywords! {
                    s,
                    abstract,
                    alignof,
                    as,
                    become,
                    box,
                    break,
                    const,
                    continue,
                    crate,
                    do,
                    else,
                    enum,
                    extern,
                    false,
                    final,
                    fn,
                    for,
                    if,
                    impl,
                    in,
                    let,
                    loop,
                    macro,
                    match,
                    mod,
                    move,
                    mut,
                    offsetof,
                    override,
                    priv,
                    proc,
                    pub,
                    pure,
                    ref,
                    return,
                    self,
                    sizeof,
                    static,
                    struct,
                    super,
                    trait,
                    true,
                    type,
                    typeof,
                    unsafe,
                    unsized,
                    use,
                    virtual,
                    where,
                    while,
                    yield,
                }
            }
        }
    }
}

impl ToSanitizedPascalCase for str {
    fn to_sanitized_pascal_case(&self) -> Cow<str> {
        let s = self.replace(BLACKLIST_CHARS, "");

        match s.chars().next().unwrap_or('\0') {
            '0' | '1' | '2' | '3' | '4' | '5' | '6' | '7' | '8' | '9' => {
                Cow::from(format!("_{}", s.to_pascal_case()))
            }
            _ => Cow::from(s.to_pascal_case()),
        }
    }
}

pub fn respace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub struct ExpandedRegister<'a> {
    pub register: &'a Register,
    pub info: &'a RegisterInfo,
    pub name: String,
    pub offset: u32,
    pub ty: Either<String, Rc<String>>,
}

/// Takes a list of "registers", some of which may actually be register arrays,
/// and turns it into a new *sorted* (by address offset) list of registers where
/// the register arrays have been expanded.
pub fn expand(registers: &[Register]) -> Vec<ExpandedRegister> {
    let mut out = vec![];

    for r in registers {
        match *r {
            Register::Single(ref info) => {
                out.push(
                    ExpandedRegister {
                        register: r,
                        info: info,
                        name: info.name.to_sanitized_snake_case().into_owned(),
                        offset: info.address_offset,
                        ty: Either::Left(
                            info.name
                                .to_sanitized_pascal_case()
                                .into_owned(),
                        ),
                    },
                )
            }
            Register::Array(ref info, ref array_info) => {
                let has_brackets = info.name.contains("[%s]");

                let ty = if has_brackets {
                    info.name.replace("[%s]", "")
                } else {
                    info.name.replace("%s", "")
                };

                let ty = Rc::new(ty.to_sanitized_pascal_case().into_owned());

                let indices = array_info
                    .dim_index
                    .as_ref()
                    .map(|v| Cow::from(&**v))
                    .unwrap_or_else(
                        || {
                            Cow::from(
                                (0..array_info.dim)
                                    .map(|i| i.to_string())
                                    .collect::<Vec<_>>(),
                            )
                        },
                    );

                for (idx, i) in indices.iter().zip(0..) {
                    let name = if has_brackets {
                        info.name.replace("[%s]", idx)
                    } else {
                        info.name.replace("%s", idx)
                    };

                    let offset = info.address_offset +
                                 i * array_info.dim_increment;

                    out.push(
                        ExpandedRegister {
                            register: r,
                            info: info,
                            name: name.to_sanitized_snake_case().into_owned(),
                            offset: offset,
                            ty: Either::Right(ty.clone()),
                        },
                    );
                }
            }
        }
    }

    out.sort_by_key(|x| x.offset);

    out
}

pub fn name_of(register: &Register) -> Cow<str> {
    match *register {
        Register::Single(ref info) => Cow::from(&*info.name),
        Register::Array(ref info, _) => {
            if info.name.contains("[%s]") {
                info.name.replace("[%s]", "").into()
            } else {
                info.name.replace("%s", "").into()
            }
        }
    }
}

// FIXME: Highly unlikely but if the register itself has extended on the base,
//        the fields can be different.
pub fn access_of(register: &Register, base: Option<&Register>) -> Access {
    register
        .access
        .or(base.and_then(|b| b.access))
        .unwrap_or_else(|| {
                let bf = base.as_ref().and_then(|b| b.fields.clone());
                let empty = vec![];
                let fields: Vec<&Field> = register.fields.as_ref().unwrap_or(&empty).iter().chain(bf.iter().flat_map(|v| v)).collect();
                if fields.len() == 0 {
                    return Access::ReadWrite
                }
                if fields.iter().all(|f| f.access == Some(Access::ReadOnly)) {
                    Access::ReadOnly
                } else if fields
                              .iter()
                              .all(|f| f.access == Some(Access::WriteOnly),) {
                    Access::WriteOnly
                } else {
                    Access::ReadWrite
                }
            }
        )
}

/// Turns `n` into an unsuffixed literal
pub fn unsuffixed(n: u64) -> Lit {
    Lit::Int(n, IntTy::Unsuffixed)
}

#[derive(Clone, Debug)]
pub struct Base<'a> {
    pub register: Option<&'a str>,
    pub field: &'a str,
}

pub fn lookup<'a>
    (
    evs: &'a [EnumeratedValues],
    fields: &'a [Field],
    register: &'a Register,
    all_registers: &'a [Register],
    peripheral: &'a Peripheral,
    usage: Usage,
) -> Result<Option<(&'a EnumeratedValues, Option<Base<'a>>)>> {
    let evs = evs.iter()
        .map(
            |evs| if let Some(ref base) = evs.derived_from {
                let mut parts = base.split('.');

                match (parts.next(), parts.next(), parts.next()) {
                    (Some(base_register), Some(base_field), Some(base_evs)) => {
                        lookup_in_peripheral(
                            base_register,
                            base_field,
                            base_evs,
                            all_registers,
                            peripheral,
                        )
                    }
                    (Some(base_field), Some(base_evs), None) => {
                        lookup_in_fields(base_evs, base_field, fields, register)
                    }
                    (Some(base_evs), None, None) => {
                        lookup_in_register(base_evs, register)
                    }
                    _ => unreachable!(),
                }
            } else {
                Ok((evs, None))
            },
        )
        .collect::<Result<Vec<_>>>()?;

    for &(ref evs, ref base) in evs.iter() {
        if evs.usage == Some(usage) {
            return Ok(Some((*evs, base.clone())));
        }
    }

    Ok(evs.first().cloned())
}

fn lookup_in_fields<'f>(
    base_evs: &str,
    base_field: &str,
    fields: &'f [Field],
    register: &Register,
) -> Result<(&'f EnumeratedValues, Option<Base<'f>>)> {
    if let Some(base_field) = fields.iter().find(|f| f.name == base_field) {
        return lookup_in_field(base_evs, None, base_field);
    } else {
        Err(
            format!(
                "Field {} not found in register {}",
                base_field,
                register.name
            ),
        )?
    }
}

fn lookup_in_peripheral<'p>
    (
    base_register: &'p str,
    base_field: &str,
    base_evs: &str,
    all_registers: &'p [Register],
    peripheral: &'p Peripheral,
) -> Result<(&'p EnumeratedValues, Option<Base<'p>>)> {
    if let Some(register) = all_registers.iter().find(
        |r| {
            r.name == base_register
        },
    ) {
        if let Some(field) = register
               .fields
               .as_ref()
               .map(|fs| &**fs)
               .unwrap_or(&[])
               .iter()
               .find(|f| f.name == base_field) {
            lookup_in_field(base_evs, Some(base_register), field)
        } else {
            Err(
                format!(
                    "No field {} in register {}",
                    base_field,
                    register.name
                ),
            )?
        }
    } else {
        Err(
            format!(
                "No register {} in peripheral {}",
                base_register,
                peripheral.name
            ),
        )?
    }
}

fn lookup_in_field<'f>(
    base_evs: &str,
    base_register: Option<&'f str>,
    field: &'f Field,
) -> Result<(&'f EnumeratedValues, Option<Base<'f>>)> {
    for evs in &field.enumerated_values {
        if evs.name.as_ref().map(|s| &**s) == Some(base_evs) {
            return Ok(
                ((evs,
                  Some(
                    Base {
                        field: &field.name,
                        register: base_register,
                    },
                ))),
            );
        }
    }

    Err(format!("No EnumeratedValues {} in field {}", base_evs, field.name),)?
}

fn lookup_in_register<'r>
    (
    base_evs: &str,
    register: &'r Register,
) -> Result<(&'r EnumeratedValues, Option<Base<'r>>)> {
    let mut matches = vec![];

    for f in register.fields.as_ref().map(|v| &**v).unwrap_or(&[]) {
        if let Some(evs) =
            f.enumerated_values
                .iter()
                .find(|evs| evs.name.as_ref().map(|s| &**s) == Some(base_evs)) {
            matches.push((evs, &f.name))
        }
    }

    match matches.first() {
        None => {
            Err(
                format!(
                    "EnumeratedValues {} not found in register {}",
                    base_evs,
                    register.name
                ),
            )?
        }
        Some(&(evs, field)) => {
            if matches.len() == 1 {
                return Ok(
                    (evs,
                     Some(
                        Base {
                            field: field,
                            register: None,
                        },
                    )),
                );
            } else {
                let fields = matches
                    .iter()
                    .map(|&(ref f, _)| &f.name)
                    .collect::<Vec<_>>();
                Err(
                    format!(
                        "Fields {:?} have an \
                                             enumeratedValues named {}",
                        fields,
                        base_evs
                    ),
                )?
            }
        }
    }
}


pub fn lookup_register<'r>(base: &str, all_registers: &'r [Register]) -> Result<&'r Register> {
    for register in all_registers {
        if register.name == base {
            return Ok(register);
        }
    }
    Err(
        format!(
            "Register {} not found",
            base,
        ),
    )?
}

// TODO: Take a Fn -> &'r T to enable
//       lookup_parents(register, all_registers, |t| t.description)
//       So we travel down until we get a Some(str)
pub fn lookup_parent<'r>(register: &'r Register, all_registers: &'r [Register]) -> Result<&'r Register> {
    match register.derived_from {
        Some(ref base) if &register.name == base => {
            Err( format!("Register {} derives from itself.", register.name))?
        }
        Some(ref base) => {
            ::util::lookup_register(base, all_registers).chain_err(|| format!("While getting base for register {}", register.name))
        }
        None => {
            Err( format!("Couldn't get base of register {}", register.name))?
        }
    }
}

pub fn description_of<'r>(register: &'r Register, base: Option<&'r Register>, all_registers: &'r [Register]) -> Result<&'r str> {
    match register.description {
        Some(ref desc) => {
            Ok(&desc)
        }
        None => {
            match base {
                Some(ref base_register) => {
                    ::util::description_of(base_register, ::util::lookup_parent(base_register, all_registers).ok(), all_registers).chain_err(|| format!("While getting description of register {}", base_register.name))
                }
                None => {
                    Err(
                        format!("Register {} has no description", register.name)
                    )?
                }
            }
        }
    }
}

pub trait U32Ext {
    fn to_ty(&self) -> Result<Ident>;
}

impl U32Ext for u32 {
    fn to_ty(&self) -> Result<Ident> {
        Ok(
            match *self {
                1...8 => Ident::new("u8"),
                9...16 => Ident::new("u16"),
                17...32 => Ident::new("u32"),
                _ => {
                    Err(
                        format!(
                            "can't convert {} bits into a Rust integer type",
                            *self
                        ),
                    )?
                }
            },
        )
    }
}
