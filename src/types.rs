//! Semantic types, user type definitions, and the unification engine.

use std::collections::HashMap;

/// Index into `Defs::types`.
pub type DefId = u32;

/// The prelude enums, registered first and always present.
pub const OPTION_DEF: DefId = 0;
pub const RESULT_DEF: DefId = 1;
pub const OPTION_SOME: u32 = 0;
pub const OPTION_NONE: u32 = 1;
pub const RESULT_OK: u32 = 0;
pub const RESULT_ERR: u32 = 1;

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Int,
    Float,
    Bool,
    Str,
    Unit,
    Range,
    /// Packed byte buffer (v0.7): binary file I/O, wire formats.
    Bytes,
    /// A worker handle (v0.7): an OS-thread isolate joined by string
    /// channels. Opaque — sendable/receivable/joinable, nothing else.
    Worker,
    /// A window handle (v0.8, Linux-only for now): an OS window + GL
    /// context (the `window` namespace). Opaque, like `Worker`.
    Window,
    List(Box<Type>),
    Map(Box<Type>, Box<Type>),
    Tuple(Vec<Type>),
    Fn(Vec<Type>, Box<Type>),
    /// A struct or enum instantiation: `Named(def, type_args)`.
    Named(DefId, Vec<Type>),
    /// A rigid generic parameter (of the enclosing declaration or of a native
    /// method scheme). Unifies only with itself.
    Param(u32),
    /// An inference variable.
    Var(u32),
}

impl Type {
    pub fn option(t: Type) -> Type {
        Type::Named(OPTION_DEF, vec![t])
    }

    pub fn contains_fn(&self, defs: &Defs) -> bool {
        self.contains_fn_inner(defs, 0)
    }

    #[allow(clippy::only_used_in_recursion)]
    fn contains_fn_inner(&self, defs: &Defs, depth: u32) -> bool {
        if depth > 32 {
            return false; // recursive type: cut off (fields re-checked at their own use sites)
        }
        match self {
            Type::Fn(..) => true,
            Type::List(t) => t.contains_fn_inner(defs, depth + 1),
            Type::Map(k, v) => {
                k.contains_fn_inner(defs, depth + 1) || v.contains_fn_inner(defs, depth + 1)
            }
            Type::Tuple(ts) => ts.iter().any(|t| t.contains_fn_inner(defs, depth + 1)),
            Type::Named(def, args) => {
                // Only inspect the type arguments; a recursive struct body could
                // loop, and function-valued fields still panic at runtime.
                let _ = def;
                args.iter().any(|t| t.contains_fn_inner(defs, depth + 1))
            }
            _ => false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StructDef {
    /// Visible to importing modules (`pub struct`). Always true for the
    /// prelude and irrelevant within the defining module.
    pub is_pub: bool,
    pub name: String,
    pub generics: Vec<String>,
    /// Field types may contain `Type::Param(i)` referring to `generics[i]`.
    pub fields: Vec<(String, Type)>,
}

#[derive(Debug, Clone)]
pub struct EnumDef {
    pub is_pub: bool,
    pub name: String,
    pub generics: Vec<String>,
    pub variants: Vec<Variant>,
}

#[derive(Debug, Clone)]
pub struct Variant {
    pub name: String,
    pub fields: Vec<Type>,
}

#[derive(Debug, Clone)]
pub enum TypeDef {
    Struct(StructDef),
    Enum(EnumDef),
}

impl TypeDef {
    pub fn name(&self) -> &str {
        match self {
            TypeDef::Struct(s) => &s.name,
            TypeDef::Enum(e) => &e.name,
        }
    }

    pub fn generics(&self) -> &[String] {
        match self {
            TypeDef::Struct(s) => &s.generics,
            TypeDef::Enum(e) => &e.generics,
        }
    }
}

/// Registry of user-defined (plus prelude) struct and enum types.
#[derive(Debug, Clone)]
pub struct Defs {
    pub types: Vec<TypeDef>,
    pub by_name: HashMap<String, DefId>,
}

impl Default for Defs {
    fn default() -> Self {
        Self::new()
    }
}

impl Defs {
    /// A registry pre-populated with the prelude `Option` and `Result` enums.
    pub fn new() -> Defs {
        let mut defs = Defs { types: Vec::new(), by_name: HashMap::new() };
        let option = defs.add(TypeDef::Enum(EnumDef {
            is_pub: true,
            name: "Option".into(),
            generics: vec!["T".into()],
            variants: vec![
                Variant { name: "Some".into(), fields: vec![Type::Param(0)] },
                Variant { name: "None".into(), fields: vec![] },
            ],
        }));
        debug_assert_eq!(option, OPTION_DEF);
        let result = defs.add(TypeDef::Enum(EnumDef {
            is_pub: true,
            name: "Result".into(),
            generics: vec!["T".into(), "E".into()],
            variants: vec![
                Variant { name: "Ok".into(), fields: vec![Type::Param(0)] },
                Variant { name: "Err".into(), fields: vec![Type::Param(1)] },
            ],
        }));
        debug_assert_eq!(result, RESULT_DEF);
        defs
    }

    pub fn add(&mut self, def: TypeDef) -> DefId {
        let id = self.types.len() as DefId;
        self.by_name.insert(def.name().to_string(), id);
        self.types.push(def);
        id
    }

    pub fn get(&self, id: DefId) -> &TypeDef {
        &self.types[id as usize]
    }

    pub fn lookup(&self, name: &str) -> Option<DefId> {
        self.by_name.get(name).copied()
    }

}

/// Substitute `Param(i)` with `args[i]` throughout `t`.
pub fn substitute(t: &Type, args: &[Type]) -> Type {
    match t {
        Type::Param(i) => args.get(*i as usize).cloned().unwrap_or(Type::Unit),
        Type::List(e) => Type::List(Box::new(substitute(e, args))),
        Type::Map(k, v) => Type::Map(Box::new(substitute(k, args)), Box::new(substitute(v, args))),
        Type::Tuple(ts) => Type::Tuple(ts.iter().map(|t| substitute(t, args)).collect()),
        Type::Fn(ps, r) => Type::Fn(
            ps.iter().map(|t| substitute(t, args)).collect(),
            Box::new(substitute(r, args)),
        ),
        Type::Named(d, ts) => Type::Named(*d, ts.iter().map(|t| substitute(t, args)).collect()),
        other => other.clone(),
    }
}

// ---------------------------------------------------------------------------
// Unification
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
pub struct Unifier {
    bindings: Vec<Option<Type>>,
}

/// A unification failure, carrying the two (resolved) types that clashed.
#[derive(Debug)]
pub struct UnifyError {
    pub left: Type,
    pub right: Type,
}

impl Unifier {
    pub fn new() -> Unifier {
        Unifier::default()
    }

    pub fn fresh(&mut self) -> Type {
        self.bindings.push(None);
        Type::Var(self.bindings.len() as u32 - 1)
    }

    pub fn fresh_id(&mut self) -> u32 {
        self.bindings.push(None);
        self.bindings.len() as u32 - 1
    }

    /// Follow variable bindings one level (the result's head is not a bound Var).
    pub fn shallow_resolve(&self, t: &Type) -> Type {
        let mut t = t.clone();
        while let Type::Var(v) = t {
            match &self.bindings[v as usize] {
                Some(bound) => t = bound.clone(),
                None => return Type::Var(v),
            }
        }
        t
    }

    /// Fully resolve a type, replacing all bound variables recursively.
    pub fn zonk(&self, t: &Type) -> Type {
        match self.shallow_resolve(t) {
            Type::List(e) => Type::List(Box::new(self.zonk(&e))),
            Type::Map(k, v) => Type::Map(Box::new(self.zonk(&k)), Box::new(self.zonk(&v))),
            Type::Tuple(ts) => Type::Tuple(ts.iter().map(|t| self.zonk(t)).collect()),
            Type::Fn(ps, r) => {
                Type::Fn(ps.iter().map(|t| self.zonk(t)).collect(), Box::new(self.zonk(&r)))
            }
            Type::Named(d, ts) => Type::Named(d, ts.iter().map(|t| self.zonk(t)).collect()),
            other => other,
        }
    }

    pub fn is_fully_resolved(&self, t: &Type) -> bool {
        match self.shallow_resolve(t) {
            Type::Var(_) => false,
            Type::List(e) => self.is_fully_resolved(&e),
            Type::Map(k, v) => self.is_fully_resolved(&k) && self.is_fully_resolved(&v),
            Type::Tuple(ts) => ts.iter().all(|t| self.is_fully_resolved(t)),
            Type::Fn(ps, r) => {
                ps.iter().all(|t| self.is_fully_resolved(t)) && self.is_fully_resolved(&r)
            }
            Type::Named(_, ts) => ts.iter().all(|t| self.is_fully_resolved(t)),
            _ => true,
        }
    }

    fn occurs(&self, var: u32, t: &Type) -> bool {
        match self.shallow_resolve(t) {
            Type::Var(v) => v == var,
            Type::List(e) => self.occurs(var, &e),
            Type::Map(k, v) => self.occurs(var, &k) || self.occurs(var, &v),
            Type::Tuple(ts) => ts.iter().any(|t| self.occurs(var, t)),
            Type::Fn(ps, r) => ps.iter().any(|t| self.occurs(var, t)) || self.occurs(var, &r),
            Type::Named(_, ts) => ts.iter().any(|t| self.occurs(var, t)),
            _ => false,
        }
    }

    /// Unify two types, binding variables as needed.
    pub fn unify(&mut self, a: &Type, b: &Type) -> Result<(), UnifyError> {
        let a = self.shallow_resolve(a);
        let b = self.shallow_resolve(b);
        match (&a, &b) {
            (Type::Var(v), Type::Var(w)) if v == w => Ok(()),
            (Type::Var(v), _) => {
                if self.occurs(*v, &b) {
                    return Err(UnifyError { left: self.zonk(&a), right: self.zonk(&b) });
                }
                self.bindings[*v as usize] = Some(b);
                Ok(())
            }
            (_, Type::Var(w)) => {
                if self.occurs(*w, &a) {
                    return Err(UnifyError { left: self.zonk(&a), right: self.zonk(&b) });
                }
                self.bindings[*w as usize] = Some(a);
                Ok(())
            }
            (Type::Int, Type::Int)
            | (Type::Float, Type::Float)
            | (Type::Bool, Type::Bool)
            | (Type::Str, Type::Str)
            | (Type::Unit, Type::Unit)
            | (Type::Range, Type::Range)
            | (Type::Bytes, Type::Bytes)
            | (Type::Worker, Type::Worker)
            | (Type::Window, Type::Window) => Ok(()),
            (Type::Param(i), Type::Param(j)) if i == j => Ok(()),
            (Type::List(x), Type::List(y)) => self.unify(x, y),
            (Type::Map(k1, v1), Type::Map(k2, v2)) => {
                self.unify(k1, k2)?;
                self.unify(v1, v2)
            }
            (Type::Tuple(xs), Type::Tuple(ys)) if xs.len() == ys.len() => {
                for (x, y) in xs.iter().zip(ys) {
                    self.unify(x, y)?;
                }
                Ok(())
            }
            (Type::Fn(ps1, r1), Type::Fn(ps2, r2)) if ps1.len() == ps2.len() => {
                for (x, y) in ps1.iter().zip(ps2) {
                    self.unify(x, y)?;
                }
                self.unify(r1, r2)
            }
            (Type::Named(d1, a1), Type::Named(d2, a2)) if d1 == d2 && a1.len() == a2.len() => {
                for (x, y) in a1.iter().zip(a2) {
                    self.unify(x, y)?;
                }
                Ok(())
            }
            _ => Err(UnifyError { left: self.zonk(&a), right: self.zonk(&b) }),
        }
    }
}

// ---------------------------------------------------------------------------
// Pretty printing
// ---------------------------------------------------------------------------

/// Render a type for diagnostics. Unresolved inference variables print as `_`,
/// rigid parameters print by name when the defining scope's names are given.
pub fn display_type(t: &Type, defs: &Defs, param_names: &[String]) -> String {
    match t {
        Type::Int => "Int".into(),
        Type::Float => "Float".into(),
        Type::Bool => "Bool".into(),
        Type::Str => "String".into(),
        Type::Unit => "Unit".into(),
        Type::Range => "Range".into(),
        Type::Bytes => "Bytes".into(),
        Type::Worker => "Worker".into(),
        Type::Window => "Window".into(),
        Type::Var(_) => "_".into(),
        Type::Param(i) => param_names
            .get(*i as usize)
            .cloned()
            .unwrap_or_else(|| format!("T{i}")),
        Type::List(e) => format!("List[{}]", display_type(e, defs, param_names)),
        Type::Map(k, v) => format!(
            "Map[{}, {}]",
            display_type(k, defs, param_names),
            display_type(v, defs, param_names)
        ),
        Type::Tuple(ts) => {
            let inner: Vec<String> = ts.iter().map(|t| display_type(t, defs, param_names)).collect();
            format!("({})", inner.join(", "))
        }
        Type::Fn(ps, r) => {
            let inner: Vec<String> = ps.iter().map(|t| display_type(t, defs, param_names)).collect();
            let ret = display_type(r, defs, param_names);
            if **r == Type::Unit {
                format!("fn({})", inner.join(", "))
            } else {
                format!("fn({}) -> {}", inner.join(", "), ret)
            }
        }
        Type::Named(d, args) => {
            let name = defs.get(*d).name();
            if args.is_empty() {
                name.to_string()
            } else {
                let inner: Vec<String> =
                    args.iter().map(|t| display_type(t, defs, param_names)).collect();
                format!("{}[{}]", name, inner.join(", "))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unify_basics() {
        let mut u = Unifier::new();
        let v = u.fresh();
        u.unify(&v, &Type::Int).unwrap();
        assert_eq!(u.zonk(&v), Type::Int);
        assert!(u.unify(&Type::Int, &Type::Float).is_err());
    }

    #[test]
    fn unify_nested() {
        let mut u = Unifier::new();
        let v = u.fresh();
        let a = Type::List(Box::new(v.clone()));
        let b = Type::List(Box::new(Type::Str));
        u.unify(&a, &b).unwrap();
        assert_eq!(u.zonk(&v), Type::Str);
    }

    #[test]
    fn occurs_check() {
        let mut u = Unifier::new();
        let v = u.fresh();
        let l = Type::List(Box::new(v.clone()));
        assert!(u.unify(&v, &l).is_err());
    }

    #[test]
    fn params_are_rigid() {
        let mut u = Unifier::new();
        assert!(u.unify(&Type::Param(0), &Type::Param(0)).is_ok());
        assert!(u.unify(&Type::Param(0), &Type::Param(1)).is_err());
        assert!(u.unify(&Type::Param(0), &Type::Int).is_err());
    }

    #[test]
    fn display() {
        let defs = Defs::new();
        let t = Type::Fn(
            vec![Type::List(Box::new(Type::Int))],
            Box::new(Type::option(Type::Str)),
        );
        assert_eq!(display_type(&t, &defs, &[]), "fn(List[Int]) -> Option[String]");
    }
}
