//! Pattern-matrix analysis: exhaustiveness and reachability, after Maranget's
//! "Warnings for pattern matching" (usefulness algorithm).
//!
//! The checker lowers surface patterns to [`DPat`] (or-patterns expanded into
//! multiple rows, struct patterns normalized to all fields in definition order)
//! and asks two questions:
//! - is a wildcard row *useful* after all unguarded arms? (→ non-exhaustive,
//!   with a witness of an uncovered value)
//! - is each arm useful w.r.t. the unguarded arms before it? (→ unreachable-arm
//!   warning otherwise)

use crate::types::{substitute, DefId, Defs, Type, TypeDef};

/// A deconstructed pattern: `None` ctor is a wildcard (bindings included).
#[derive(Debug, Clone)]
pub struct DPat {
    pub ctor: Option<Ctor>,
    pub args: Vec<DPat>,
}

impl DPat {
    pub fn wild() -> DPat {
        DPat { ctor: None, args: Vec::new() }
    }

    pub fn ctor(c: Ctor, args: Vec<DPat>) -> DPat {
        DPat { ctor: Some(c), args }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Ctor {
    Unit,
    Bool(bool),
    Int(i64),
    /// Bit representation, so `-0.0` and `NaN` compare like the runtime does not.
    FloatBits(u64),
    Str(String),
    Tuple(usize),
    Variant(DefId, u32),
    Struct(DefId),
}

/// The set of constructors a type can produce, if finitely enumerable.
fn complete_ctors(ty: &Type, defs: &Defs) -> Option<Vec<Ctor>> {
    match ty {
        Type::Bool => Some(vec![Ctor::Bool(false), Ctor::Bool(true)]),
        Type::Unit => Some(vec![Ctor::Unit]),
        Type::Tuple(ts) => Some(vec![Ctor::Tuple(ts.len())]),
        Type::Named(d, _) => match defs.get(*d) {
            TypeDef::Enum(e) => Some(
                (0..e.variants.len() as u32).map(|i| Ctor::Variant(*d, i)).collect(),
            ),
            TypeDef::Struct(_) => Some(vec![Ctor::Struct(*d)]),
        },
        // Int/Float/String have effectively-infinite domains; Range, functions,
        // lists, maps, and unresolved types are opaque (only `_` covers them).
        _ => None,
    }
}

/// The types of a constructor's sub-patterns, given the scrutinee column type.
fn ctor_arg_types(ctor: &Ctor, ty: &Type, defs: &Defs) -> Vec<Type> {
    match (ctor, ty) {
        (Ctor::Tuple(_), Type::Tuple(ts)) => ts.clone(),
        (Ctor::Variant(d, v), Type::Named(d2, args)) if d == d2 => {
            match defs.get(*d) {
                TypeDef::Enum(e) => e.variants[*v as usize]
                    .fields
                    .iter()
                    .map(|f| substitute(f, args))
                    .collect(),
                _ => Vec::new(),
            }
        }
        (Ctor::Struct(d), Type::Named(d2, args)) if d == d2 => match defs.get(*d) {
            TypeDef::Struct(s) => s.fields.iter().map(|(_, f)| substitute(f, args)).collect(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn ctor_arity(ctor: &Ctor, defs: &Defs) -> usize {
    match ctor {
        Ctor::Tuple(n) => *n,
        Ctor::Variant(d, v) => match defs.get(*d) {
            TypeDef::Enum(e) => e.variants[*v as usize].fields.len(),
            _ => 0,
        },
        Ctor::Struct(d) => match defs.get(*d) {
            TypeDef::Struct(s) => s.fields.len(),
            _ => 0,
        },
        _ => 0,
    }
}

/// Specialize a row by `ctor`: if the row's head matches (or is a wildcard),
/// return the row with the head replaced by its sub-patterns.
fn specialize_row(row: &[DPat], ctor: &Ctor, arity: usize) -> Option<Vec<DPat>> {
    let head = &row[0];
    match &head.ctor {
        Some(c) if c == ctor => {
            let mut out = head.args.clone();
            out.extend_from_slice(&row[1..]);
            Some(out)
        }
        Some(_) => None,
        None => {
            let mut out = vec![DPat::wild(); arity];
            out.extend_from_slice(&row[1..]);
            Some(out)
        }
    }
}

/// Rows whose head is a wildcard, with the head removed.
fn default_matrix(matrix: &[Vec<DPat>]) -> Vec<Vec<DPat>> {
    matrix
        .iter()
        .filter(|row| row[0].ctor.is_none())
        .map(|row| row[1..].to_vec())
        .collect()
}

/// Is `row` useful with respect to `matrix`? Returns a witness (a value shape
/// matched by `row` but by no row of `matrix`) when it is.
///
/// `types[i]` is the type of column `i`; used to enumerate complete ctor sets.
pub fn usefulness(
    matrix: &[Vec<DPat>],
    row: &[DPat],
    types: &[Type],
    defs: &Defs,
) -> Option<Vec<DPat>> {
    if row.is_empty() {
        return if matrix.is_empty() { Some(Vec::new()) } else { None };
    }
    if types.len() < row.len() {
        // Misaligned after an upstream type error (e.g. a pattern whose ctor
        // doesn't fit the column type): bail out conservatively.
        return None;
    }
    let head = &row[0];
    let col_ty = &types[0];

    match &head.ctor {
        Some(ctor) => {
            let arity = ctor_arity(ctor, defs);
            let spec: Vec<Vec<DPat>> = matrix
                .iter()
                .filter_map(|r| specialize_row(r, ctor, arity))
                .collect();
            let mut sub_row = head.args.clone();
            sub_row.extend_from_slice(&row[1..]);
            let mut sub_types = ctor_arg_types(ctor, col_ty, defs);
            sub_types.extend_from_slice(&types[1..]);
            let witness = usefulness(&spec, &sub_row, &sub_types, defs)?;
            Some(reassemble(ctor.clone(), arity, witness))
        }
        None => {
            let present: Vec<Ctor> = {
                let mut seen: Vec<Ctor> = Vec::new();
                for r in matrix {
                    if let Some(c) = &r[0].ctor {
                        if !seen.contains(c) {
                            seen.push(c.clone());
                        }
                    }
                }
                seen
            };
            let complete = complete_ctors(col_ty, defs);

            match complete {
                Some(all) if all.iter().all(|c| present.contains(c)) && !all.is_empty() => {
                    // The column's ctors are completely covered: try each.
                    for ctor in &all {
                        let arity = ctor_arity(ctor, defs);
                        let spec: Vec<Vec<DPat>> = matrix
                            .iter()
                            .filter_map(|r| specialize_row(r, ctor, arity))
                            .collect();
                        let mut sub_row = vec![DPat::wild(); arity];
                        sub_row.extend_from_slice(&row[1..]);
                        let mut sub_types = ctor_arg_types(ctor, col_ty, defs);
                        sub_types.extend_from_slice(&types[1..]);
                        if let Some(w) = usefulness(&spec, &sub_row, &sub_types, defs) {
                            return Some(reassemble(ctor.clone(), arity, w));
                        }
                    }
                    None
                }
                _ => {
                    // Incomplete (or infinite) head column: recurse on the
                    // default matrix.
                    let dm = default_matrix(matrix);
                    let witness = usefulness(&dm, &row[1..], &types[1..], defs)?;
                    // Build the head witness: prefer a concrete missing ctor.
                    let head_witness = match &complete {
                        Some(all) => {
                            match all.iter().find(|c| !present.contains(c)) {
                                Some(missing) => {
                                    let ar = ctor_arity(missing, defs);
                                    DPat::ctor(missing.clone(), vec![DPat::wild(); ar])
                                }
                                None => DPat::wild(),
                            }
                        }
                        None => missing_scalar_witness(&present, col_ty),
                    };
                    let mut out = vec![head_witness];
                    out.extend(witness);
                    Some(out)
                }
            }
        }
    }
}

/// For infinite scalar domains, produce an example value not present in the
/// matched literals (falls back to `_`).
fn missing_scalar_witness(present: &[Ctor], ty: &Type) -> DPat {
    match ty {
        Type::Int => {
            let mut candidate: i64 = 0;
            loop {
                if !present.iter().any(|c| matches!(c, Ctor::Int(i) if *i == candidate)) {
                    return DPat::ctor(Ctor::Int(candidate), Vec::new());
                }
                candidate = if candidate >= 0 { -(candidate + 1) } else { -candidate };
                if candidate > 1000 {
                    return DPat::wild();
                }
            }
        }
        Type::Str => {
            for candidate in ["\"\"", "\"a\"", "\"b\"", "\"c\""] {
                let inner = candidate.trim_matches('"');
                if !present.iter().any(|c| matches!(c, Ctor::Str(s) if s == inner)) {
                    return DPat::ctor(Ctor::Str(inner.to_string()), Vec::new());
                }
            }
            DPat::wild()
        }
        _ => DPat::wild(),
    }
}

fn reassemble(ctor: Ctor, arity: usize, mut witness: Vec<DPat>) -> Vec<DPat> {
    let rest = witness.split_off(arity);
    let mut out = vec![DPat::ctor(ctor, witness)];
    out.extend(rest);
    out
}

/// Render a witness pattern for a diagnostic, e.g. `Shape.Circle(_)`.
pub fn display_pattern(p: &DPat, defs: &Defs) -> String {
    match &p.ctor {
        None => "_".into(),
        Some(Ctor::Unit) => "()".into(),
        Some(Ctor::Bool(b)) => b.to_string(),
        Some(Ctor::Int(i)) => i.to_string(),
        Some(Ctor::FloatBits(bits)) => format!("{:?}", f64::from_bits(*bits)),
        Some(Ctor::Str(s)) => format!("{s:?}"),
        Some(Ctor::Tuple(_)) => {
            let inner: Vec<String> = p.args.iter().map(|a| display_pattern(a, defs)).collect();
            format!("({})", inner.join(", "))
        }
        Some(Ctor::Variant(d, v)) => {
            let TypeDef::Enum(e) = defs.get(*d) else { return "_".into() };
            let vname = &e.variants[*v as usize].name;
            let prelude = matches!(e.name.as_str(), "Option" | "Result");
            let head = if prelude { vname.clone() } else { format!("{}.{}", e.name, vname) };
            if p.args.is_empty() {
                head
            } else {
                let inner: Vec<String> =
                    p.args.iter().map(|a| display_pattern(a, defs)).collect();
                format!("{}({})", head, inner.join(", "))
            }
        }
        Some(Ctor::Struct(d)) => {
            let name = defs.get(*d).name();
            if p.args.iter().all(|a| a.ctor.is_none()) {
                format!("{name} {{ .. }}")
            } else {
                let TypeDef::Struct(s) = defs.get(*d) else { return format!("{name} {{ .. }}") };
                let inner: Vec<String> = s
                    .fields
                    .iter()
                    .zip(&p.args)
                    .map(|((fname, _), a)| format!("{fname}: {}", display_pattern(a, defs)))
                    .collect();
                format!("{} {{ {} }}", name, inner.join(", "))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EnumDef, Variant, OPTION_DEF};

    fn defs() -> Defs {
        let mut d = Defs::new();
        d.add(TypeDef::Enum(EnumDef {
            is_pub: true,
            name: "Shape".into(),
            generics: vec![],
            variants: vec![
                Variant { name: "Circle".into(), fields: vec![Type::Float] },
                Variant { name: "Rect".into(), fields: vec![Type::Float, Type::Float] },
                Variant { name: "Empty".into(), fields: vec![] },
            ],
        }));
        d
    }

    fn shape_ty(defs: &Defs) -> Type {
        Type::Named(defs.lookup("Shape").unwrap(), vec![])
    }

    #[test]
    fn missing_variant_is_witnessed() {
        let defs = defs();
        let d = defs.lookup("Shape").unwrap();
        // match s { Circle(_) -> .., Rect(_, _) -> .. }  — missing Empty
        let matrix = vec![
            vec![DPat::ctor(Ctor::Variant(d, 0), vec![DPat::wild()])],
            vec![DPat::ctor(Ctor::Variant(d, 1), vec![DPat::wild(), DPat::wild()])],
        ];
        let w = usefulness(&matrix, &[DPat::wild()], &[shape_ty(&defs)], &defs).unwrap();
        assert_eq!(display_pattern(&w[0], &defs), "Shape.Empty");
    }

    #[test]
    fn full_enum_coverage_is_exhaustive() {
        let defs = defs();
        let d = defs.lookup("Shape").unwrap();
        let matrix = vec![
            vec![DPat::ctor(Ctor::Variant(d, 0), vec![DPat::wild()])],
            vec![DPat::ctor(Ctor::Variant(d, 1), vec![DPat::wild(), DPat::wild()])],
            vec![DPat::ctor(Ctor::Variant(d, 2), vec![])],
        ];
        assert!(usefulness(&matrix, &[DPat::wild()], &[shape_ty(&defs)], &defs).is_none());
    }

    #[test]
    fn bool_coverage() {
        let defs = Defs::new();
        let matrix = vec![
            vec![DPat::ctor(Ctor::Bool(true), vec![])],
        ];
        let w = usefulness(&matrix, &[DPat::wild()], &[Type::Bool], &defs).unwrap();
        assert_eq!(display_pattern(&w[0], &defs), "false");

        let full = vec![
            vec![DPat::ctor(Ctor::Bool(true), vec![])],
            vec![DPat::ctor(Ctor::Bool(false), vec![])],
        ];
        assert!(usefulness(&full, &[DPat::wild()], &[Type::Bool], &defs).is_none());
    }

    #[test]
    fn int_literals_never_exhaust() {
        let defs = Defs::new();
        let matrix = vec![
            vec![DPat::ctor(Ctor::Int(0), vec![])],
            vec![DPat::ctor(Ctor::Int(1), vec![])],
        ];
        let w = usefulness(&matrix, &[DPat::wild()], &[Type::Int], &defs).unwrap();
        // Witness picks a concrete uncovered integer.
        assert_eq!(display_pattern(&w[0], &defs), "-1");
    }

    #[test]
    fn nested_option_witness() {
        let defs = Defs::new();
        let ty = Type::Named(OPTION_DEF, vec![Type::Bool]);
        // match o { Some(true) -> .., None -> .. } — missing Some(false)
        let matrix = vec![
            vec![DPat::ctor(Ctor::Variant(OPTION_DEF, 0), vec![DPat::ctor(Ctor::Bool(true), vec![])])],
            vec![DPat::ctor(Ctor::Variant(OPTION_DEF, 1), vec![])],
        ];
        let w = usefulness(&matrix, &[DPat::wild()], &[ty], &defs).unwrap();
        assert_eq!(display_pattern(&w[0], &defs), "Some(false)");
    }

    #[test]
    fn unreachable_detection() {
        let defs = Defs::new();
        // arm `_` then arm `0`: the latter is not useful.
        let matrix = vec![vec![DPat::wild()]];
        let row = vec![DPat::ctor(Ctor::Int(0), vec![])];
        assert!(usefulness(&matrix, &row, &[Type::Int], &defs).is_none());
    }

    #[test]
    fn tuple_decomposition() {
        let defs = Defs::new();
        let ty = Type::Tuple(vec![Type::Bool, Type::Bool]);
        // (true, _), (_, true), (false, false) — exhaustive
        let matrix = vec![
            vec![DPat::ctor(Ctor::Tuple(2), vec![DPat::ctor(Ctor::Bool(true), vec![]), DPat::wild()])],
            vec![DPat::ctor(Ctor::Tuple(2), vec![DPat::wild(), DPat::ctor(Ctor::Bool(true), vec![])])],
            vec![DPat::ctor(
                Ctor::Tuple(2),
                vec![DPat::ctor(Ctor::Bool(false), vec![]), DPat::ctor(Ctor::Bool(false), vec![])],
            )],
        ];
        assert!(usefulness(&matrix, &[DPat::wild()], std::slice::from_ref(&ty), &defs).is_none());

        // remove the last row: witness (false, false)
        let w = usefulness(&matrix[..2], &[DPat::wild()], &[ty], &defs).unwrap();
        assert_eq!(display_pattern(&w[0], &defs), "(false, false)");
    }
}
