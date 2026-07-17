//! The native (builtin) function registry: names, receivers, and type schemes.
//!
//! This module is pure data — the VM implements execution in `vm::call_native`
//! with a `match` over [`Native`]. The type checker uses [`method`], [`free_fn`],
//! and [`math_member`] to resolve names and [`Native::sig`] for typing.
//!
//! Scheme convention (see `NativeSig`): `Param(0)`/`Param(1)` are the receiver's
//! type arguments (`List[T]` → P0; `Map[K, V]` → P0, P1; `Option[T]` → P0;
//! `Result[T, E]` → P0, P1). Method-own generics start at `Param(4)` (e.g. the
//! `U` in `List[T].map[U]`).

use crate::types::Type;

/// Every native function and method in the language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Native {
    // Free functions
    Print,
    Println,
    Str,
    Panic,
    Assert,
    AssertEq,
    Clock,
    Input,
    TryCall,
    /// `char(code)` — the one-character string for a Unicode scalar value.
    CharFromCode,
    /// `bytes(n)` — a zero-filled byte buffer (v0.7).
    BytesNew,
    /// `bytes_of(list)` — a byte buffer from a List[Int] of 0..255 values.
    BytesOf,
    BytesLen,
    BytesGet,
    BytesSet,
    BytesPush,
    BytesPushU16le,
    BytesPushI16le,
    BytesPushU32le,
    /// Bulk appends (v0.7 efficiency pass): a whole buffer (snapshot
    /// semantics — self-append works) and a string's UTF-8 bytes.
    BytesPushBytes,
    BytesPushStr,
    /// Big-endian pushers — same range checks as the LE trio.
    BytesPushU16be,
    BytesPushU32be,
    /// 64-bit pushers (v0.8): no range check needed — `Int` already IS the
    /// 64-bit two's-complement value, so its own bit pattern is what gets
    /// written (one push, not a u64/i64 pair: at 64 bits the two are the
    /// same operation, unlike push_u16le/push_i16le's differing ranges).
    BytesPushU64le,
    BytesPushU64be,
    /// Multi-byte readers at a byte offset; OOB panics match `get`.
    BytesReadU16le,
    BytesReadI16le,
    BytesReadU32le,
    BytesReadU16be,
    BytesReadU32be,
    /// 64-bit readers (v0.8): the 8 bytes reinterpreted as `Int` directly.
    BytesReadU64le,
    BytesReadU64be,
    /// 32-bit float pushers/readers (v0.8): `Float` is `f64`; these narrow
    /// to `f32` at the boundary — the wire format graphics/audio data
    /// actually uses (vertex attributes, uniforms, WAV/PCM samples, ...).
    BytesPushF32le,
    BytesPushF32be,
    BytesReadF32le,
    BytesReadF32be,
    BytesSlice,
    BytesConcat,
    BytesToList,
    BytesUtf8,
    StrToBytes,
    // math namespace
    MathSqrt,
    MathSin,
    MathCos,
    MathTan,
    MathAtan,
    MathAtan2,
    MathLog,
    MathLog2,
    MathExp,
    MathPow,
    MathFloor,
    MathCeil,
    MathRound,
    MathAbsInt,
    MathAbs,
    MathMin,
    MathMax,
    MathMinFloat,
    MathMaxFloat,
    MathRandom,
    MathSeed,
    MathRandInt,
    MathLog10,
    MathFmod,

    // fs.* — file system access (v0.3). Fallible operations return
    // Result[_, String] with the OS error message.
    FsRead,
    FsWrite,
    FsAppend,
    FsExists,
    FsIsDir,
    FsListDir,
    FsCreateDir,
    FsRemove,
    FsReadBytes,
    FsWriteBytes,
    // fft.* (v0.7)
    FftFft,
    FftIfft,
    FftRfft,
    /// `fft.magnitude(re, im)` (v0.8): every `rfft` consumer wrote the same
    /// `re.zip(im).map(|p| sqrt(p.0*p.0 + p.1*p.1))` line.
    FftMagnitude,
    // worker.* + Worker handle methods (v0.7)
    WorkerSpawn,
    WorkerSelfSend,
    WorkerSelfRecv,
    WorkerIsWorker,
    WorkerHandleSend,
    WorkerHandleRecv,
    WorkerHandleJoin,
    /// Non-blocking `recv` (v0.8): `Option[Option[String]]` — outer `None`
    /// means no message ready right now, distinct from the inner `None`
    /// (the worker finished) that a blocking `recv` also reports.
    WorkerHandleTryRecv,
    WorkerSelfTryRecv,

    // os.* — process environment (v0.3).
    OsArgs,
    OsEnv,
    OsRun,
    OsExit,
    OsTime,

    // gpu.* — compute-shader dispatch (v0.7, experimental). The natives are
    // always registered; without the `gpu` cargo feature they degrade
    // gracefully (see src/gpu.rs).
    GpuAvailable,
    GpuAdapterInfo,
    GpuRun,
    /// `gpu.run_spirv(spirv, input, out_len, wx, wy, wz)` (v0.9):
    /// `gpu.run`'s `Bytes`-shader sibling — SPIR-V is a binary format, so
    /// the blob rides the buffer type instead of masquerading as text. A
    /// sibling rather than an overload for the same reason as
    /// `window.create_metal` (Fable has neither default parameters nor
    /// overloading). Ingested natively by the vulkan backend; other
    /// backends report which entry point they want instead.
    GpuRunSpirv,
    /// `gpu.backend()` (v0.9): `"metal"` | `"wgpu"` | `"vulkan"` | `"none"` — which
    /// implementation `gpu.run` dispatches to in this build. The `gpu`
    /// analog of `win.backend_name()`: programs branch on it to pick the
    /// shader dialect (MSL vs. WGSL).
    GpuBackend,

    // window.* + Window handle methods (v0.8, Linux-only for now). The
    // natives are always registered; without the `gl` cargo feature they
    // degrade gracefully (see src/window/mod.rs).
    WindowCreate,
    /// `window.create_metal(title, w, h)` (v0.9, macOS aarch64 only): a
    /// Metal-backed sibling of `window.create` — additive alongside the
    /// OpenGL/CGL path, never a replacement (see CLAUDE.md's standing
    /// exception). Without the `metal` cargo feature it degrades
    /// gracefully, same as `WindowCreate` without `gl`.
    WindowCreateMetal,
    /// Vulkan-backed sibling of `window.create` (Linux/X11), riding the
    /// same `vulkan` cargo feature as `gpu.run_spirv`'s compute backend.
    /// Without it (or off Linux) it degrades gracefully, same as
    /// `WindowCreateMetal` off Apple Silicon macOS.
    WindowCreateVulkan,
    WindowHandlePoll,
    WindowHandleShouldClose,
    WindowHandleClose,
    WindowHandleKeyDown,
    WindowHandleMousePos,
    WindowHandleWidth,
    WindowHandleHeight,
    WindowHandleClear,
    WindowHandleSwapBuffers,
    /// `win.make_current()` (v0.8): binds this window's GL context as the
    /// one every subsequent `gfx.*` native operates against (mirrors
    /// `glfwMakeContextCurrent`) — see `Vm::gfx_current_window`.
    WindowHandleMakeCurrent,
    /// `win.backend_name()` (v0.9): `"opengl"` | `"metal"` — the one
    /// deliberate escape hatch for backend-specific behavior (shader
    /// source text is inherently GLSL vs. MSL); everything else about the
    /// `gfx.*` call shape stays identical across backends.
    WindowHandleBackendName,

    // gfx.* (v0.8, feature-gated on `gl`, same as `window`) — OpenGL 3.3
    // core-profile draw calls against "whichever window is currently
    // current" (`win.make_current()`, above). The natives are always
    // registered; without the `gl` feature, or before any window has ever
    // called `make_current()`, they degrade gracefully (see
    // `natives::gfx_window` and `src/window/mod.rs`).
    GfxCompileProgram,
    GfxUseProgram,
    GfxDeleteProgram,
    GfxCreateBuffer,
    GfxDeleteBuffer,
    GfxBindBuffer,
    GfxUploadBuffer,
    GfxCreateVertexArray,
    GfxBindVertexArray,
    GfxDeleteVertexArray,
    GfxSetVertexAttrib,
    GfxDisableVertexAttrib,
    GfxCreateTexture,
    GfxDeleteTexture,
    GfxBindTexture,
    GfxActiveTextureUnit,
    GfxUploadTexture,
    GfxSetUniformInt,
    GfxSetUniformFloat,
    GfxSetUniformVec2,
    GfxSetUniformVec3,
    GfxSetUniformVec4,
    /// `gfx.set_uniform_mat4(p, name, m)`: `m`'s scheme type is `Param(0)`
    /// (a fresh type variable), not a concrete `Mat4` — natives can't name
    /// a `std` module struct's `DefId` (`std.glm.Mat4` isn't assigned one
    /// until `std.glm` is imported). Passing anything but a real `Mat4`
    /// (4 `Vec4` fields, each 4 `Float`s) is a runtime panic, not a type
    /// error (see `natives::mat4_arg`).
    GfxSetUniformMat4,
    GfxDrawArrays,
    GfxDrawElements,
    GfxClear,
    GfxSetDepthTest,
    GfxViewport,
    GfxReadPixels,

    // List methods
    ListLen,
    ListIsEmpty,
    ListPush,
    ListPop,
    ListInsert,
    ListRemove,
    ListGet,
    ListFirst,
    ListLast,
    ListContains,
    ListIndexOf,
    ListReverse,
    ListSort,
    ListSortBy,
    ListMap,
    ListFilter,
    ListEach,
    ListFold,
    ListAny,
    ListAll,
    ListFind,
    ListFlatMap,
    ListZip,
    ListEnumerate,
    ListSlice,
    ListConcat,
    ListJoin,
    ListClone,
    ListClear,
    // String methods
    StrLen,
    StrByteLen,
    StrIsEmpty,
    StrChars,
    StrSplit,
    StrTrim,
    StrToUpper,
    StrToLower,
    StrContains,
    StrStartsWith,
    StrEndsWith,
    StrReplace,
    StrSlice,
    StrCharAt,
    StrCodeAt,
    StrTrimStart,
    StrTrimEnd,
    StrIndexOfFrom,
    StrIndexOf,
    StrRepeat,
    StrPadLeft,
    StrPadRight,
    StrParseInt,
    StrParseFloat,
    StrParseHex,
    StrToString,
    // Map methods
    MapLen,
    MapIsEmpty,
    MapGet,
    MapInsert,
    MapRemove,
    MapContainsKey,
    MapKeys,
    MapValues,
    MapEntries,
    MapClear,
    MapClone,
    // Int methods
    IntToFloat,
    IntToString,
    IntAbs,
    IntPow,
    IntMin,
    IntMax,
    /// Bit intrinsics (v0.7 efficiency pass) over the 64-bit
    /// two's-complement pattern; both zero-count methods return 64 for 0.
    IntCountOnes,
    IntLeadingZeros,
    IntTrailingZeros,
    /// Logical (zero-filling) right shift — `>>`'s panic contract.
    IntUshr,
    /// Rotates take their count mod 64 and never panic.
    IntRotateLeft,
    IntRotateRight,
    /// Lowercase minimal hex of the two's-complement bit pattern.
    IntToHex,
    /// Wrapping (v0.8) arithmetic — the checked `+ - *` operators panic on
    /// overflow; these truncate to the low 64 bits instead, for hash
    /// finalizers and the like. A 32-bit wrap is just `.wrapping_mul(y) &
    /// 0xFFFFFFFF` — one primitive covers both widths, no separate 32-bit
    /// intrinsic needed.
    IntWrappingAdd,
    IntWrappingSub,
    IntWrappingMul,
    // Float methods
    FloatToInt,
    FloatToString,
    FloatAbs,
    FloatFloor,
    FloatCeil,
    FloatRound,
    FloatSqrt,
    FloatIsNan,
    FloatToFixed,
    // Option methods
    OptIsSome,
    OptIsNone,
    OptUnwrap,
    OptUnwrapOr,
    OptMap,
    OptAndThen,
    OptOr,
    // Result methods
    ResIsOk,
    ResIsErr,
    ResUnwrap,
    ResUnwrapOr,
    ResUnwrapErr,
    ResMap,
    ResMapErr,
    ResAndThen,
    // Range methods
    RangeToList,
    RangeContains,
    RangeLen,
    RangeMap,
    RangeFilter,
    RangeEach,
    RangeFold,
    RangeRev,
    /// Short-circuiting (v0.8): stop at the first true (`any`) / false
    /// (`all`), like `List`'s. Previously only reachable via `.to_list()`.
    RangeAny,
    RangeAll,
}

/// The type scheme of a native. `params` excludes the receiver for methods.
/// `generics` is how many scheme parameters are used in total (receiver args
/// and method-own); the checker instantiates `Param(0)..Param(generics)` — with
/// method-own params living at indices 4+ — as fresh inference variables.
pub struct NativeSig {
    pub params: Vec<Type>,
    pub ret: Type,
    /// Highest `Param` index used, plus one (0 for fully monomorphic).
    pub max_param: u32,
}

/// Receiver kind for method lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Recv {
    Int,
    Float,
    Str,
    Bytes,
    Worker,
    Window,
    List,
    Map,
    Range,
    Option_,
    Result_,
}

impl Recv {
    pub fn describe(self) -> &'static str {
        match self {
            Recv::Int => "Int",
            Recv::Float => "Float",
            Recv::Str => "String",
            Recv::Bytes => "Bytes",
            Recv::Worker => "Worker",
            Recv::Window => "Window",
            Recv::List => "List",
            Recv::Map => "Map",
            Recv::Range => "Range",
            Recv::Option_ => "Option",
            Recv::Result_ => "Result",
        }
    }
}

// Scheme parameter shorthands.
fn p0() -> Type {
    Type::Param(0)
}
fn p1() -> Type {
    Type::Param(1)
}
fn p4() -> Type {
    Type::Param(4)
}
fn list(t: Type) -> Type {
    Type::List(Box::new(t))
}
fn map_(k: Type, v: Type) -> Type {
    Type::Map(Box::new(k), Box::new(v))
}
fn opt(t: Type) -> Type {
    Type::Named(crate::types::OPTION_DEF, vec![t])
}
fn res(t: Type, e: Type) -> Type {
    Type::Named(crate::types::RESULT_DEF, vec![t, e])
}
fn func(params: Vec<Type>, ret: Type) -> Type {
    Type::Fn(params, Box::new(ret))
}
fn tup(ts: Vec<Type>) -> Type {
    Type::Tuple(ts)
}

use Type::{Bool, Float, Int, Str as TStr, Unit};

impl Native {
    /// Resolve a method by receiver kind and name.
    pub fn method(recv: Recv, name: &str) -> Option<Native> {
        METHOD_TABLE
            .iter()
            .find(|(r, n, _)| *r == recv && *n == name)
            .map(|(_, _, v)| *v)
    }

    /// Every builtin method name for a receiver kind (for tooling —
    /// completion in the language server).
    pub fn methods_of(recv: Recv) -> impl Iterator<Item = &'static str> {
        METHOD_TABLE
            .iter()
            .filter(move |(r, _, _)| *r == recv)
            .map(|(_, n, _)| *n)
    }

    /// Resolve a free (prelude) function by name.
    pub fn free_fn(name: &str) -> Option<Native> {
        use Native::*;
        Some(match name {
            "print" => Print,
            "println" => Println,
            "str" => Str,
            "panic" => Panic,
            "assert" => Assert,
            "assert_eq" => AssertEq,
            "clock" => Clock,
            "input" => Input,
            "try" => TryCall,
            "char" => CharFromCode,
            "bytes" => BytesNew,
            "bytes_of" => BytesOf,
            _ => return None,
        })
    }

    /// Is `name` a builtin namespace (usable only as `name.member`)?
    pub fn is_namespace(name: &str) -> bool {
        matches!(name, "math" | "fs" | "os" | "fft" | "worker" | "gpu" | "window" | "gfx")
    }

    /// Resolve `<ns>.<member>` for any builtin namespace.
    pub fn namespace_member(ns: &str, member: &str) -> Option<MathMember> {
        use Native::*;
        match ns {
            "math" => Self::math_member(member),
            "fs" => Some(MathMember::Fn(match member {
                "read" => FsRead,
                "write" => FsWrite,
                "append" => FsAppend,
                "exists" => FsExists,
                "is_dir" => FsIsDir,
                "list_dir" => FsListDir,
                "create_dir" => FsCreateDir,
                "remove" => FsRemove,
                "read_bytes" => FsReadBytes,
                "write_bytes" => FsWriteBytes,
                _ => return None,
            })),
            "fft" => Some(MathMember::Fn(match member {
                "fft" => FftFft,
                "ifft" => FftIfft,
                "rfft" => FftRfft,
                "magnitude" => FftMagnitude,
                _ => return None,
            })),
            "worker" => Some(MathMember::Fn(match member {
                "spawn" => WorkerSpawn,
                "send" => WorkerSelfSend,
                "recv" => WorkerSelfRecv,
                "try_recv" => WorkerSelfTryRecv,
                "is_worker" => WorkerIsWorker,
                _ => return None,
            })),
            "os" => Some(MathMember::Fn(match member {
                "args" => OsArgs,
                "env" => OsEnv,
                "run" => OsRun,
                "exit" => OsExit,
                "time" => OsTime,
                _ => return None,
            })),
            "gpu" => Some(MathMember::Fn(match member {
                "available" => GpuAvailable,
                "adapter_info" => GpuAdapterInfo,
                "run" => GpuRun,
                "run_spirv" => GpuRunSpirv,
                "backend" => GpuBackend,
                _ => return None,
            })),
            "window" => Some(MathMember::Fn(match member {
                // Only `create`/`create_metal`/`create_vulkan` are
                // namespace-level free functions; the rest are methods on
                // the `Window` receiver (METHOD_TABLE).
                "create" => WindowCreate,
                "create_metal" => WindowCreateMetal,
                "create_vulkan" => WindowCreateVulkan,
                _ => return None,
            })),
            "gfx" => Some(MathMember::Fn(match member {
                "compile_program" => GfxCompileProgram,
                "use_program" => GfxUseProgram,
                "delete_program" => GfxDeleteProgram,
                "create_buffer" => GfxCreateBuffer,
                "delete_buffer" => GfxDeleteBuffer,
                "bind_buffer" => GfxBindBuffer,
                "upload_buffer" => GfxUploadBuffer,
                "create_vertex_array" => GfxCreateVertexArray,
                "bind_vertex_array" => GfxBindVertexArray,
                "delete_vertex_array" => GfxDeleteVertexArray,
                "set_vertex_attrib" => GfxSetVertexAttrib,
                "disable_vertex_attrib" => GfxDisableVertexAttrib,
                "create_texture" => GfxCreateTexture,
                "delete_texture" => GfxDeleteTexture,
                "bind_texture" => GfxBindTexture,
                "active_texture_unit" => GfxActiveTextureUnit,
                "upload_texture" => GfxUploadTexture,
                "set_uniform_int" => GfxSetUniformInt,
                "set_uniform_float" => GfxSetUniformFloat,
                "set_uniform_vec2" => GfxSetUniformVec2,
                "set_uniform_vec3" => GfxSetUniformVec3,
                "set_uniform_vec4" => GfxSetUniformVec4,
                "set_uniform_mat4" => GfxSetUniformMat4,
                "draw_arrays" => GfxDrawArrays,
                "draw_elements" => GfxDrawElements,
                "clear" => GfxClear,
                "set_depth_test" => GfxSetDepthTest,
                "viewport" => GfxViewport,
                "read_pixels" => GfxReadPixels,
                _ => return None,
            })),
            _ => None,
        }
    }

    /// Every member name of a builtin namespace (for completion). A unit
    /// test asserts each listed name resolves via `namespace_member`.
    pub fn namespace_members(ns: &str) -> &'static [&'static str] {
        match ns {
            "math" => &[
                "pi", "e", "sqrt", "sin", "cos", "tan", "atan", "atan2", "log", "log2",
                "log10", "exp", "pow", "fmod", "floor", "ceil", "round", "abs_int", "abs",
                "min", "max", "min_float", "max_float", "random", "seed", "rand_int",
            ],
            "fs" => &[
                "read", "write", "append", "exists", "is_dir", "list_dir", "create_dir",
                "remove", "read_bytes", "write_bytes",
            ],
            "os" => &["args", "env", "run", "exit", "time"],
            "fft" => &["fft", "ifft", "rfft"],
            "worker" => &["spawn", "send", "recv", "is_worker"],
            "gpu" => &["available", "adapter_info", "run"],
            "window" => &["create"],
            "gfx" => &[
                "compile_program",
                "use_program",
                "delete_program",
                "create_buffer",
                "delete_buffer",
                "bind_buffer",
                "upload_buffer",
                "create_vertex_array",
                "bind_vertex_array",
                "delete_vertex_array",
                "set_vertex_attrib",
                "disable_vertex_attrib",
                "create_texture",
                "delete_texture",
                "bind_texture",
                "active_texture_unit",
                "upload_texture",
                "set_uniform_int",
                "set_uniform_float",
                "set_uniform_vec2",
                "set_uniform_vec3",
                "set_uniform_vec4",
                "set_uniform_mat4",
                "draw_arrays",
                "draw_elements",
                "clear",
                "set_depth_test",
                "viewport",
                "read_pixels",
            ],
            _ => &[],
        }
    }

    /// Resolve a `math.<name>` member. Returns either a function or a constant.
    pub fn math_member(name: &str) -> Option<MathMember> {
        use Native::*;
        Some(match name {
            "pi" => MathMember::Const(std::f64::consts::PI),
            "e" => MathMember::Const(std::f64::consts::E),
            "sqrt" => MathMember::Fn(MathSqrt),
            "sin" => MathMember::Fn(MathSin),
            "cos" => MathMember::Fn(MathCos),
            "tan" => MathMember::Fn(MathTan),
            "atan" => MathMember::Fn(MathAtan),
            "atan2" => MathMember::Fn(MathAtan2),
            "log" => MathMember::Fn(MathLog),
            "log2" => MathMember::Fn(MathLog2),
            "exp" => MathMember::Fn(MathExp),
            "pow" => MathMember::Fn(MathPow),
            "floor" => MathMember::Fn(MathFloor),
            "ceil" => MathMember::Fn(MathCeil),
            "round" => MathMember::Fn(MathRound),
            "abs_int" => MathMember::Fn(MathAbsInt),
            "abs" => MathMember::Fn(MathAbs),
            "min" => MathMember::Fn(MathMin),
            "max" => MathMember::Fn(MathMax),
            "min_float" => MathMember::Fn(MathMinFloat),
            "max_float" => MathMember::Fn(MathMaxFloat),
            "random" => MathMember::Fn(MathRandom),
            "seed" => MathMember::Fn(MathSeed),
            "rand_int" => MathMember::Fn(MathRandInt),
            "log10" => MathMember::Fn(MathLog10),
            "fmod" => MathMember::Fn(MathFmod),
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        use Native::*;
        match self {
            Print => "print",
            Println => "println",
            Str => "str",
            Panic => "panic",
            Assert => "assert",
            AssertEq => "assert_eq",
            Clock => "clock",
            Input => "input",
            TryCall => "try",
            CharFromCode => "char",
            BytesNew => "bytes",
            BytesOf => "bytes_of",
            BytesLen => "len",
            BytesGet => "get",
            BytesSet => "set",
            BytesPush => "push",
            BytesPushU16le => "push_u16le",
            BytesPushI16le => "push_i16le",
            BytesPushU32le => "push_u32le",
            BytesPushBytes => "push_bytes",
            BytesPushStr => "push_str",
            BytesPushU16be => "push_u16be",
            BytesPushU32be => "push_u32be",
            BytesPushU64le => "push_u64le",
            BytesPushU64be => "push_u64be",
            BytesReadU16le => "read_u16le",
            BytesReadI16le => "read_i16le",
            BytesReadU32le => "read_u32le",
            BytesReadU16be => "read_u16be",
            BytesReadU32be => "read_u32be",
            BytesReadU64le => "read_u64le",
            BytesReadU64be => "read_u64be",
            BytesPushF32le => "push_f32le",
            BytesPushF32be => "push_f32be",
            BytesReadF32le => "read_f32le",
            BytesReadF32be => "read_f32be",
            BytesSlice => "slice",
            BytesConcat => "concat",
            BytesToList => "to_list",
            BytesUtf8 => "utf8",
            StrToBytes => "to_bytes",
            MathSqrt => "math.sqrt",
            MathSin => "math.sin",
            MathCos => "math.cos",
            MathTan => "math.tan",
            MathAtan => "math.atan",
            MathAtan2 => "math.atan2",
            MathLog => "math.log",
            MathLog2 => "math.log2",
            MathExp => "math.exp",
            MathPow => "math.pow",
            MathFloor => "math.floor",
            MathCeil => "math.ceil",
            MathRound => "math.round",
            MathAbsInt => "math.abs_int",
            MathAbs => "math.abs",
            MathMin => "math.min",
            MathMax => "math.max",
            MathMinFloat => "math.min_float",
            MathMaxFloat => "math.max_float",
            MathRandom => "math.random",
            MathSeed => "math.seed",
            MathRandInt => "math.rand_int",
            MathLog10 => "math.log10",
            MathFmod => "math.fmod",
            FsRead => "fs.read",
            FsWrite => "fs.write",
            FsAppend => "fs.append",
            FsExists => "fs.exists",
            FsIsDir => "fs.is_dir",
            FsListDir => "fs.list_dir",
            FsCreateDir => "fs.create_dir",
            FsReadBytes => "fs.read_bytes",
            FsWriteBytes => "fs.write_bytes",
            WorkerSpawn => "worker.spawn",
            WorkerSelfSend => "worker.send",
            WorkerSelfRecv => "worker.recv",
            WorkerSelfTryRecv => "worker.try_recv",
            WorkerIsWorker => "worker.is_worker",
            WorkerHandleSend => "send",
            WorkerHandleRecv => "recv",
            WorkerHandleTryRecv => "try_recv",
            WorkerHandleJoin => "join",
            FftFft => "fft.fft",
            FftIfft => "fft.ifft",
            FftRfft => "fft.rfft",
            FftMagnitude => "fft.magnitude",
            FsRemove => "fs.remove",
            OsArgs => "os.args",
            OsEnv => "os.env",
            OsRun => "os.run",
            OsExit => "os.exit",
            OsTime => "os.time",
            GpuAvailable => "gpu.available",
            GpuAdapterInfo => "gpu.adapter_info",
            GpuRun => "gpu.run",
            GpuRunSpirv => "gpu.run_spirv",
            GpuBackend => "gpu.backend",
            WindowCreate => "window.create",
            WindowCreateMetal => "window.create_metal",
            WindowCreateVulkan => "window.create_vulkan",
            WindowHandlePoll => "poll",
            WindowHandleShouldClose => "should_close",
            WindowHandleClose => "close",
            WindowHandleKeyDown => "key_down",
            WindowHandleMousePos => "mouse_pos",
            WindowHandleWidth => "width",
            WindowHandleHeight => "height",
            WindowHandleClear => "clear",
            WindowHandleSwapBuffers => "swap_buffers",
            WindowHandleMakeCurrent => "make_current",
            WindowHandleBackendName => "backend_name",
            GfxCompileProgram => "gfx.compile_program",
            GfxUseProgram => "gfx.use_program",
            GfxDeleteProgram => "gfx.delete_program",
            GfxCreateBuffer => "gfx.create_buffer",
            GfxDeleteBuffer => "gfx.delete_buffer",
            GfxBindBuffer => "gfx.bind_buffer",
            GfxUploadBuffer => "gfx.upload_buffer",
            GfxCreateVertexArray => "gfx.create_vertex_array",
            GfxBindVertexArray => "gfx.bind_vertex_array",
            GfxDeleteVertexArray => "gfx.delete_vertex_array",
            GfxSetVertexAttrib => "gfx.set_vertex_attrib",
            GfxDisableVertexAttrib => "gfx.disable_vertex_attrib",
            GfxCreateTexture => "gfx.create_texture",
            GfxDeleteTexture => "gfx.delete_texture",
            GfxBindTexture => "gfx.bind_texture",
            GfxActiveTextureUnit => "gfx.active_texture_unit",
            GfxUploadTexture => "gfx.upload_texture",
            GfxSetUniformInt => "gfx.set_uniform_int",
            GfxSetUniformFloat => "gfx.set_uniform_float",
            GfxSetUniformVec2 => "gfx.set_uniform_vec2",
            GfxSetUniformVec3 => "gfx.set_uniform_vec3",
            GfxSetUniformVec4 => "gfx.set_uniform_vec4",
            GfxSetUniformMat4 => "gfx.set_uniform_mat4",
            GfxDrawArrays => "gfx.draw_arrays",
            GfxDrawElements => "gfx.draw_elements",
            GfxClear => "gfx.clear",
            GfxSetDepthTest => "gfx.set_depth_test",
            GfxViewport => "gfx.viewport",
            GfxReadPixels => "gfx.read_pixels",
            ListLen => "len",
            ListIsEmpty => "is_empty",
            ListPush => "push",
            ListPop => "pop",
            ListInsert => "insert",
            ListRemove => "remove",
            ListGet => "get",
            ListFirst => "first",
            ListLast => "last",
            ListContains => "contains",
            ListIndexOf => "index_of",
            ListReverse => "reverse",
            ListSort => "sort",
            ListSortBy => "sort_by",
            ListMap => "map",
            ListFilter => "filter",
            ListEach => "each",
            ListFold => "fold",
            ListAny => "any",
            ListAll => "all",
            ListFind => "find",
            ListFlatMap => "flat_map",
            ListZip => "zip",
            ListEnumerate => "enumerate",
            ListSlice => "slice",
            ListConcat => "concat",
            ListJoin => "join",
            ListClone => "clone",
            ListClear => "clear",
            StrLen => "len",
            StrByteLen => "byte_len",
            StrIsEmpty => "is_empty",
            StrChars => "chars",
            StrSplit => "split",
            StrTrim => "trim",
            StrToUpper => "to_upper",
            StrToLower => "to_lower",
            StrContains => "contains",
            StrStartsWith => "starts_with",
            StrEndsWith => "ends_with",
            StrReplace => "replace",
            StrSlice => "slice",
            StrCharAt => "char_at",
            StrCodeAt => "code_at",
            StrTrimStart => "trim_start",
            StrTrimEnd => "trim_end",
            StrIndexOfFrom => "index_of_from",
            StrIndexOf => "index_of",
            StrRepeat => "repeat",
            StrPadLeft => "pad_left",
            StrPadRight => "pad_right",
            StrParseInt => "parse_int",
            StrParseFloat => "parse_float",
            StrParseHex => "parse_hex",
            StrToString => "to_string",
            MapLen => "len",
            MapIsEmpty => "is_empty",
            MapGet => "get",
            MapInsert => "insert",
            MapRemove => "remove",
            MapContainsKey => "contains_key",
            MapKeys => "keys",
            MapValues => "values",
            MapEntries => "entries",
            MapClear => "clear",
            MapClone => "clone",
            IntToFloat => "to_float",
            IntToString => "to_string",
            IntAbs => "abs",
            IntPow => "pow",
            IntMin => "min",
            IntMax => "max",
            IntCountOnes => "count_ones",
            IntLeadingZeros => "leading_zeros",
            IntTrailingZeros => "trailing_zeros",
            IntUshr => "ushr",
            IntRotateLeft => "rotate_left",
            IntRotateRight => "rotate_right",
            IntToHex => "to_hex",
            IntWrappingAdd => "wrapping_add",
            IntWrappingSub => "wrapping_sub",
            IntWrappingMul => "wrapping_mul",
            FloatToInt => "to_int",
            FloatToString => "to_string",
            FloatAbs => "abs",
            FloatFloor => "floor",
            FloatCeil => "ceil",
            FloatRound => "round",
            FloatSqrt => "sqrt",
            FloatIsNan => "is_nan",
            FloatToFixed => "to_fixed",
            OptIsSome => "is_some",
            OptIsNone => "is_none",
            OptUnwrap => "unwrap",
            OptUnwrapOr => "unwrap_or",
            OptMap => "map",
            OptAndThen => "and_then",
            OptOr => "or",
            ResIsOk => "is_ok",
            ResIsErr => "is_err",
            ResUnwrap => "unwrap",
            ResUnwrapOr => "unwrap_or",
            ResUnwrapErr => "unwrap_err",
            ResMap => "map",
            ResMapErr => "map_err",
            ResAndThen => "and_then",
            RangeToList => "to_list",
            RangeContains => "contains",
            RangeLen => "len",
            RangeMap => "map",
            RangeFilter => "filter",
            RangeEach => "each",
            RangeFold => "fold",
            RangeRev => "rev",
            RangeAny => "any",
            RangeAll => "all",
        }
    }

    /// The type scheme. For methods, `params` excludes the receiver.
    pub fn sig(self) -> NativeSig {
        use Native::*;
        // (params, ret, max_param)
        let (params, ret, max_param): (Vec<Type>, Type, u32) = match self {
            Print | Println => (vec![p0()], Unit, 1),
            Str => (vec![p0()], TStr, 1),
            // `panic` returns a scheme variable so it typechecks anywhere.
            Panic => (vec![TStr], p0(), 1),
            Assert => (vec![Bool], Unit, 0),
            AssertEq => (vec![p0(), p0()], Unit, 1),
            Clock => (vec![], Float, 0),
            Input => (vec![], opt(TStr), 0),
            // try(f) runs f and catches runtime panics.
            TryCall => (vec![func(vec![], p0())], res(p0(), TStr), 1),

            MathSqrt | MathSin | MathCos | MathTan | MathAtan | MathLog | MathLog2 | MathLog10
            | MathExp | MathFloor | MathCeil | MathRound | MathAbs => (vec![Float], Float, 0),
            MathAtan2 | MathPow | MathMinFloat | MathMaxFloat | MathFmod => {
                (vec![Float, Float], Float, 0)
            }
            MathAbsInt => (vec![Int], Int, 0),
            MathMin | MathMax => (vec![Int, Int], Int, 0),
            MathRandom => (vec![], Float, 0),
            MathSeed => (vec![Int], Unit, 0),
            MathRandInt => (vec![Int, Int], Int, 0),
            // `char` panics on an invalid scalar value (codes normally come
            // from `code_at`, which only produces valid ones).
            CharFromCode => (vec![Int], TStr, 0),

            // Bytes (v0.7). Setters panic on out-of-range values, like list
            // indexing; the LE pushers exist because Fable has no bitwise
            // operators and wire formats shouldn't need them.
            BytesNew => (vec![Int], Type::Bytes, 0),
            BytesOf => (vec![list(Int)], Type::Bytes, 0),
            BytesLen => (vec![], Int, 0),
            BytesGet => (vec![Int], Int, 0),
            BytesSet => (vec![Int, Int], Unit, 0),
            BytesPush => (vec![Int], Unit, 0),
            BytesPushU16le | BytesPushI16le | BytesPushU32le | BytesPushU16be
            | BytesPushU32be | BytesPushU64le | BytesPushU64be => (vec![Int], Unit, 0),
            BytesPushBytes => (vec![Type::Bytes], Unit, 0),
            BytesPushStr => (vec![TStr], Unit, 0),
            BytesReadU16le | BytesReadI16le | BytesReadU32le | BytesReadU16be
            | BytesReadU32be | BytesReadU64le | BytesReadU64be => (vec![Int], Int, 0),
            BytesPushF32le | BytesPushF32be => (vec![Float], Unit, 0),
            BytesReadF32le | BytesReadF32be => (vec![Int], Float, 0),
            BytesSlice => (vec![Int, Int], Type::Bytes, 0),
            BytesConcat => (vec![Type::Bytes], Type::Bytes, 0),
            BytesToList => (vec![], list(Int), 0),
            BytesUtf8 => (vec![], res(TStr, TStr), 0),
            StrToBytes => (vec![], Type::Bytes, 0),

            FsRead => (vec![TStr], res(TStr, TStr), 0),
            FsWrite | FsAppend => (vec![TStr, TStr], res(Unit, TStr), 0),
            FsExists | FsIsDir => (vec![TStr], Bool, 0),
            FsListDir => (vec![TStr], res(list(TStr), TStr), 0),
            FsCreateDir | FsRemove => (vec![TStr], res(Unit, TStr), 0),
            FsReadBytes => (vec![TStr], res(Type::Bytes, TStr), 0),
            FsWriteBytes => (vec![TStr, Type::Bytes], res(Unit, TStr), 0),
            FftFft | FftIfft => (
                vec![list(Float), list(Float)],
                tup(vec![list(Float), list(Float)]),
                0,
            ),
            FftRfft => (vec![list(Float)], tup(vec![list(Float), list(Float)]), 0),
            FftMagnitude => (vec![list(Float), list(Float)], list(Float), 0),

            // worker.* (v0.7). Only Strings cross threads; spawn resolves
            // the file like an import (relative to the spawning script) and
            // surfaces compile errors synchronously in the Err.
            WorkerSpawn => (vec![TStr, list(TStr)], res(Type::Worker, TStr), 0),
            WorkerSelfSend => (vec![TStr], Bool, 0),
            WorkerSelfRecv => (vec![], opt(TStr), 0),
            WorkerSelfTryRecv => (vec![], opt(opt(TStr)), 0),
            WorkerIsWorker => (vec![], Bool, 0),
            WorkerHandleSend => (vec![TStr], Bool, 0),
            WorkerHandleRecv => (vec![], opt(TStr), 0),
            WorkerHandleTryRecv => (vec![], opt(opt(TStr)), 0),
            WorkerHandleJoin => (vec![], res(Unit, TStr), 0),
            OsArgs => (vec![], list(TStr), 0),
            OsEnv => (vec![TStr], opt(TStr), 0),
            OsRun => (vec![TStr, list(TStr)], res(tup(vec![Int, TStr, TStr]), TStr), 0),
            // Diverges: like `panic`, the return type is a scheme variable so
            // an exit typechecks in any value position.
            OsExit => (vec![Int], p0(), 1),
            OsTime => (vec![], Float, 0),

            GpuAvailable => (vec![], Bool, 0),
            GpuAdapterInfo => (vec![], TStr, 0),
            // gpu.run(wgsl, input, out_len, wx, wy, wz)
            GpuRun => (
                vec![TStr, Type::Bytes, Int, Int, Int, Int],
                res(Type::Bytes, TStr),
                0,
            ),
            GpuRunSpirv => (
                vec![Type::Bytes, Type::Bytes, Int, Int, Int, Int],
                res(Type::Bytes, TStr),
                0,
            ),
            GpuBackend => (vec![], TStr, 0),

            // window.* (v0.8; macOS gained a Metal-backed sibling entry
            // point in v0.9, Linux a Vulkan-backed one after that). The
            // `create*` family mirrors `worker.spawn`'s `Result[_, String]`
            // shape.
            WindowCreate => (vec![TStr, Int, Int], res(Type::Window, TStr), 0),
            WindowCreateMetal => (vec![TStr, Int, Int], res(Type::Window, TStr), 0),
            WindowCreateVulkan => (vec![TStr, Int, Int], res(Type::Window, TStr), 0),
            WindowHandlePoll => (vec![], Unit, 0),
            WindowHandleShouldClose => (vec![], Bool, 0),
            WindowHandleClose => (vec![], Unit, 0),
            WindowHandleKeyDown => (vec![TStr], Bool, 0),
            WindowHandleMousePos => (vec![], tup(vec![Float, Float]), 0),
            WindowHandleWidth | WindowHandleHeight => (vec![], Int, 0),
            WindowHandleClear => (vec![Float, Float, Float, Float], Unit, 0),
            WindowHandleSwapBuffers => (vec![], Unit, 0),
            WindowHandleMakeCurrent => (vec![], Unit, 0),
            WindowHandleBackendName => (vec![], TStr, 0),

            // gfx.* (v0.8). Only `compile_program` can meaningfully fail
            // (bad shader source); everything else assumes valid GL state
            // once a program is validly linked and bound, matching
            // `window`'s own methods' shape (no `Result` plumbing).
            GfxCompileProgram => (vec![TStr, TStr], res(Int, TStr), 0),
            GfxUseProgram => (vec![Int], Unit, 0),
            GfxDeleteProgram => (vec![Int], Unit, 0),
            GfxCreateBuffer => (vec![], Int, 0),
            GfxDeleteBuffer => (vec![Int], Unit, 0),
            GfxBindBuffer => (vec![TStr, Int], Unit, 0),
            GfxUploadBuffer => (vec![TStr, Type::Bytes, Bool], Unit, 0),
            GfxCreateVertexArray => (vec![], Int, 0),
            GfxBindVertexArray => (vec![Int], Unit, 0),
            GfxDeleteVertexArray => (vec![Int], Unit, 0),
            GfxSetVertexAttrib => (vec![Int, Int, Int, Int], Unit, 0),
            GfxDisableVertexAttrib => (vec![Int], Unit, 0),
            GfxCreateTexture => (vec![], Int, 0),
            GfxDeleteTexture => (vec![Int], Unit, 0),
            GfxBindTexture => (vec![Int], Unit, 0),
            GfxActiveTextureUnit => (vec![Int], Unit, 0),
            GfxUploadTexture => (vec![Type::Bytes, Int, Int, Bool], Unit, 0),
            GfxSetUniformInt => (vec![Int, TStr, Int], Unit, 0),
            GfxSetUniformFloat => (vec![Int, TStr, Float], Unit, 0),
            GfxSetUniformVec2 => (vec![Int, TStr, Float, Float], Unit, 0),
            GfxSetUniformVec3 => (vec![Int, TStr, Float, Float, Float], Unit, 0),
            GfxSetUniformVec4 => (vec![Int, TStr, Float, Float, Float, Float], Unit, 0),
            // `m`'s type is a fresh scheme variable, not a concrete `Mat4` —
            // see the `Native::GfxSetUniformMat4` doc comment above.
            GfxSetUniformMat4 => (vec![Int, TStr, p0()], Unit, 1),
            GfxDrawArrays => (vec![Int, Int], Unit, 0),
            GfxDrawElements => (vec![Int, Int], Unit, 0),
            GfxClear => (vec![Float, Float, Float, Float], Unit, 0),
            GfxSetDepthTest => (vec![Bool], Unit, 0),
            GfxViewport => (vec![Int, Int, Int, Int], Unit, 0),
            GfxReadPixels => (vec![Int, Int, Int, Int], Type::Bytes, 0),

            // List[T] — receiver args at P0.
            ListLen => (vec![], Int, 1),
            ListIsEmpty => (vec![], Bool, 1),
            ListPush => (vec![p0()], Unit, 1),
            ListPop => (vec![], opt(p0()), 1),
            ListInsert => (vec![Int, p0()], Unit, 1),
            ListRemove => (vec![Int], p0(), 1),
            ListGet => (vec![Int], opt(p0()), 1),
            ListFirst | ListLast => (vec![], opt(p0()), 1),
            ListContains => (vec![p0()], Bool, 1),
            ListIndexOf => (vec![p0()], opt(Int), 1),
            ListReverse => (vec![], list(p0()), 1),
            ListSort => (vec![], list(p0()), 1),
            ListSortBy => (vec![func(vec![p0(), p0()], Int)], list(p0()), 1),
            ListMap => (vec![func(vec![p0()], p4())], list(p4()), 5),
            ListFilter => (vec![func(vec![p0()], Bool)], list(p0()), 1),
            ListEach => (vec![func(vec![p0()], Unit)], Unit, 1),
            ListFold => (vec![p4(), func(vec![p4(), p0()], p4())], p4(), 5),
            ListAny | ListAll => (vec![func(vec![p0()], Bool)], Bool, 1),
            ListFind => (vec![func(vec![p0()], Bool)], opt(p0()), 1),
            ListFlatMap => (vec![func(vec![p0()], list(p4()))], list(p4()), 5),
            ListZip => (vec![list(p4())], list(tup(vec![p0(), p4()])), 5),
            ListEnumerate => (vec![], list(tup(vec![Int, p0()])), 1),
            ListSlice => (vec![Int, Int], list(p0()), 1),
            ListConcat => (vec![list(p0())], list(p0()), 1),
            ListJoin => (vec![TStr], TStr, 1), // receiver constrained to List[String] at check time
            ListClone => (vec![], list(p0()), 1),
            ListClear => (vec![], Unit, 1),

            StrLen | StrByteLen => (vec![], Int, 0),
            StrIsEmpty => (vec![], Bool, 0),
            StrChars => (vec![], list(TStr), 0),
            StrSplit => (vec![TStr], list(TStr), 0),
            StrTrim | StrToUpper | StrToLower => (vec![], TStr, 0),
            StrContains | StrStartsWith | StrEndsWith => (vec![TStr], Bool, 0),
            StrReplace => (vec![TStr, TStr], TStr, 0),
            StrSlice => (vec![Int, Int], TStr, 0),
            StrCharAt => (vec![Int], opt(TStr), 0),
            StrCodeAt => (vec![Int], opt(Int), 0),
            StrTrimStart | StrTrimEnd => (vec![], TStr, 0),
            StrIndexOf => (vec![TStr], opt(Int), 0),
            StrIndexOfFrom => (vec![TStr, Int], opt(Int), 0),
            StrRepeat => (vec![Int], TStr, 0),
            StrPadLeft | StrPadRight => (vec![Int, TStr], TStr, 0),
            StrParseInt => (vec![], opt(Int), 0),
            StrParseFloat => (vec![], opt(Float), 0),
            StrParseHex => (vec![], opt(Int), 0),
            StrToString => (vec![], TStr, 0),

            // Map[K, V] — receiver args at P0 (K), P1 (V).
            MapLen => (vec![], Int, 2),
            MapIsEmpty => (vec![], Bool, 2),
            MapGet => (vec![p0()], opt(p1()), 2),
            MapInsert => (vec![p0(), p1()], opt(p1()), 2),
            MapRemove => (vec![p0()], opt(p1()), 2),
            MapContainsKey => (vec![p0()], Bool, 2),
            MapKeys => (vec![], list(p0()), 2),
            MapValues => (vec![], list(p1()), 2),
            MapEntries => (vec![], list(tup(vec![p0(), p1()])), 2),
            MapClear => (vec![], Unit, 2),
            MapClone => (vec![], map_(p0(), p1()), 2),

            IntToFloat => (vec![], Float, 0),
            IntToString => (vec![], TStr, 0),
            IntAbs => (vec![], Int, 0),
            IntPow => (vec![Int], Int, 0),
            IntMin | IntMax => (vec![Int], Int, 0),
            IntCountOnes | IntLeadingZeros | IntTrailingZeros => (vec![], Int, 0),
            IntUshr | IntRotateLeft | IntRotateRight => (vec![Int], Int, 0),
            IntToHex => (vec![], TStr, 0),
            IntWrappingAdd | IntWrappingSub | IntWrappingMul => (vec![Int], Int, 0),

            FloatToInt => (vec![], Int, 0),
            FloatToString => (vec![], TStr, 0),
            FloatAbs | FloatFloor | FloatCeil | FloatRound | FloatSqrt => (vec![], Float, 0),
            FloatIsNan => (vec![], Bool, 0),
            FloatToFixed => (vec![Int], TStr, 0),

            // Option[T] — receiver arg at P0.
            OptIsSome | OptIsNone => (vec![], Bool, 1),
            OptUnwrap => (vec![], p0(), 1),
            OptUnwrapOr => (vec![p0()], p0(), 1),
            OptMap => (vec![func(vec![p0()], p4())], opt(p4()), 5),
            OptAndThen => (vec![func(vec![p0()], opt(p4()))], opt(p4()), 5),
            OptOr => (vec![opt(p0())], opt(p0()), 1),

            // Result[T, E] — receiver args at P0 (T), P1 (E).
            ResIsOk | ResIsErr => (vec![], Bool, 2),
            ResUnwrap => (vec![], p0(), 2),
            ResUnwrapOr => (vec![p0()], p0(), 2),
            ResUnwrapErr => (vec![], p1(), 2),
            ResMap => (vec![func(vec![p0()], p4())], res(p4(), p1()), 5),
            ResMapErr => (vec![func(vec![p1()], p4())], res(p0(), p4()), 5),
            ResAndThen => (vec![func(vec![p0()], res(p4(), p1()))], res(p4(), p1()), 5),

            RangeToList => (vec![], list(Int), 0),
            RangeContains => (vec![Int], Bool, 0),
            RangeLen => (vec![], Int, 0),
            RangeMap => (vec![func(vec![Int], p4())], list(p4()), 5),
            RangeFilter => (vec![func(vec![Int], Bool)], list(Int), 0),
            RangeEach => (vec![func(vec![Int], Unit)], Unit, 0),
            RangeFold => (vec![p4(), func(vec![p4(), Int], p4())], p4(), 5),
            RangeRev => (vec![], list(Int), 0),
            RangeAny | RangeAll => (vec![func(vec![Int], Bool)], Bool, 0),
        };
        NativeSig { params, ret, max_param }
    }
}

/// A `math.<name>` member: either a native function or a float constant.
pub enum MathMember {
    Fn(Native),
    Const(f64),
}

/// The builtin method registry: (receiver kind, name, native).
/// `Native::method` and `Native::methods_of` both read this single table.
const METHOD_TABLE: &[(Recv, &str, Native)] = &[
    (Recv::List, "len", Native::ListLen),
    (Recv::List, "is_empty", Native::ListIsEmpty),
    (Recv::List, "push", Native::ListPush),
    (Recv::List, "pop", Native::ListPop),
    (Recv::List, "insert", Native::ListInsert),
    (Recv::List, "remove", Native::ListRemove),
    (Recv::List, "get", Native::ListGet),
    (Recv::List, "first", Native::ListFirst),
    (Recv::List, "last", Native::ListLast),
    (Recv::List, "contains", Native::ListContains),
    (Recv::List, "index_of", Native::ListIndexOf),
    (Recv::List, "reverse", Native::ListReverse),
    (Recv::List, "sort", Native::ListSort),
    (Recv::List, "sort_by", Native::ListSortBy),
    (Recv::List, "map", Native::ListMap),
    (Recv::List, "filter", Native::ListFilter),
    (Recv::List, "each", Native::ListEach),
    (Recv::List, "fold", Native::ListFold),
    (Recv::List, "any", Native::ListAny),
    (Recv::List, "all", Native::ListAll),
    (Recv::List, "find", Native::ListFind),
    (Recv::List, "flat_map", Native::ListFlatMap),
    (Recv::List, "zip", Native::ListZip),
    (Recv::List, "enumerate", Native::ListEnumerate),
    (Recv::List, "slice", Native::ListSlice),
    (Recv::List, "concat", Native::ListConcat),
    (Recv::List, "join", Native::ListJoin),
    (Recv::List, "clone", Native::ListClone),
    (Recv::List, "clear", Native::ListClear),
    (Recv::Str, "len", Native::StrLen),
    (Recv::Str, "byte_len", Native::StrByteLen),
    (Recv::Str, "is_empty", Native::StrIsEmpty),
    (Recv::Str, "chars", Native::StrChars),
    (Recv::Str, "split", Native::StrSplit),
    (Recv::Str, "trim", Native::StrTrim),
    (Recv::Str, "trim_start", Native::StrTrimStart),
    (Recv::Str, "trim_end", Native::StrTrimEnd),
    (Recv::Str, "to_upper", Native::StrToUpper),
    (Recv::Str, "to_lower", Native::StrToLower),
    (Recv::Str, "contains", Native::StrContains),
    (Recv::Str, "starts_with", Native::StrStartsWith),
    (Recv::Str, "ends_with", Native::StrEndsWith),
    (Recv::Str, "replace", Native::StrReplace),
    (Recv::Str, "slice", Native::StrSlice),
    (Recv::Str, "char_at", Native::StrCharAt),
    (Recv::Str, "code_at", Native::StrCodeAt),
    (Recv::Str, "index_of", Native::StrIndexOf),
    (Recv::Str, "index_of_from", Native::StrIndexOfFrom),
    (Recv::Str, "repeat", Native::StrRepeat),
    (Recv::Str, "pad_left", Native::StrPadLeft),
    (Recv::Str, "pad_right", Native::StrPadRight),
    (Recv::Str, "parse_int", Native::StrParseInt),
    (Recv::Str, "parse_float", Native::StrParseFloat),
    (Recv::Str, "parse_hex", Native::StrParseHex),
    (Recv::Str, "to_string", Native::StrToString),
    (Recv::Map, "len", Native::MapLen),
    (Recv::Map, "is_empty", Native::MapIsEmpty),
    (Recv::Map, "get", Native::MapGet),
    (Recv::Map, "insert", Native::MapInsert),
    (Recv::Map, "remove", Native::MapRemove),
    (Recv::Map, "contains_key", Native::MapContainsKey),
    (Recv::Map, "keys", Native::MapKeys),
    (Recv::Map, "values", Native::MapValues),
    (Recv::Map, "entries", Native::MapEntries),
    (Recv::Map, "clear", Native::MapClear),
    (Recv::Map, "clone", Native::MapClone),
    (Recv::Int, "to_float", Native::IntToFloat),
    (Recv::Int, "to_string", Native::IntToString),
    (Recv::Int, "abs", Native::IntAbs),
    (Recv::Int, "pow", Native::IntPow),
    (Recv::Int, "min", Native::IntMin),
    (Recv::Int, "max", Native::IntMax),
    (Recv::Int, "count_ones", Native::IntCountOnes),
    (Recv::Int, "leading_zeros", Native::IntLeadingZeros),
    (Recv::Int, "trailing_zeros", Native::IntTrailingZeros),
    (Recv::Int, "ushr", Native::IntUshr),
    (Recv::Int, "rotate_left", Native::IntRotateLeft),
    (Recv::Int, "rotate_right", Native::IntRotateRight),
    (Recv::Int, "to_hex", Native::IntToHex),
    (Recv::Int, "wrapping_add", Native::IntWrappingAdd),
    (Recv::Int, "wrapping_sub", Native::IntWrappingSub),
    (Recv::Int, "wrapping_mul", Native::IntWrappingMul),
    (Recv::Float, "to_int", Native::FloatToInt),
    (Recv::Float, "to_string", Native::FloatToString),
    (Recv::Float, "abs", Native::FloatAbs),
    (Recv::Float, "floor", Native::FloatFloor),
    (Recv::Float, "ceil", Native::FloatCeil),
    (Recv::Float, "round", Native::FloatRound),
    (Recv::Float, "sqrt", Native::FloatSqrt),
    (Recv::Float, "is_nan", Native::FloatIsNan),
    (Recv::Float, "to_fixed", Native::FloatToFixed),
    (Recv::Str, "to_bytes", Native::StrToBytes),
    (Recv::Bytes, "len", Native::BytesLen),
    (Recv::Bytes, "get", Native::BytesGet),
    (Recv::Bytes, "set", Native::BytesSet),
    (Recv::Bytes, "push", Native::BytesPush),
    (Recv::Bytes, "push_u16le", Native::BytesPushU16le),
    (Recv::Bytes, "push_i16le", Native::BytesPushI16le),
    (Recv::Bytes, "push_u32le", Native::BytesPushU32le),
    (Recv::Bytes, "push_bytes", Native::BytesPushBytes),
    (Recv::Bytes, "push_str", Native::BytesPushStr),
    (Recv::Bytes, "push_u16be", Native::BytesPushU16be),
    (Recv::Bytes, "push_u32be", Native::BytesPushU32be),
    (Recv::Bytes, "push_u64le", Native::BytesPushU64le),
    (Recv::Bytes, "push_u64be", Native::BytesPushU64be),
    (Recv::Bytes, "read_u16le", Native::BytesReadU16le),
    (Recv::Bytes, "read_i16le", Native::BytesReadI16le),
    (Recv::Bytes, "read_u32le", Native::BytesReadU32le),
    (Recv::Bytes, "read_u16be", Native::BytesReadU16be),
    (Recv::Bytes, "read_u32be", Native::BytesReadU32be),
    (Recv::Bytes, "read_u64le", Native::BytesReadU64le),
    (Recv::Bytes, "read_u64be", Native::BytesReadU64be),
    (Recv::Bytes, "push_f32le", Native::BytesPushF32le),
    (Recv::Bytes, "push_f32be", Native::BytesPushF32be),
    (Recv::Bytes, "read_f32le", Native::BytesReadF32le),
    (Recv::Bytes, "read_f32be", Native::BytesReadF32be),
    (Recv::Bytes, "slice", Native::BytesSlice),
    (Recv::Bytes, "concat", Native::BytesConcat),
    (Recv::Bytes, "to_list", Native::BytesToList),
    (Recv::Bytes, "utf8", Native::BytesUtf8),
    (Recv::Worker, "send", Native::WorkerHandleSend),
    (Recv::Worker, "recv", Native::WorkerHandleRecv),
    (Recv::Worker, "try_recv", Native::WorkerHandleTryRecv),
    (Recv::Worker, "join", Native::WorkerHandleJoin),
    (Recv::Window, "poll", Native::WindowHandlePoll),
    (Recv::Window, "should_close", Native::WindowHandleShouldClose),
    (Recv::Window, "close", Native::WindowHandleClose),
    (Recv::Window, "key_down", Native::WindowHandleKeyDown),
    (Recv::Window, "mouse_pos", Native::WindowHandleMousePos),
    (Recv::Window, "width", Native::WindowHandleWidth),
    (Recv::Window, "height", Native::WindowHandleHeight),
    (Recv::Window, "clear", Native::WindowHandleClear),
    (Recv::Window, "swap_buffers", Native::WindowHandleSwapBuffers),
    (Recv::Window, "make_current", Native::WindowHandleMakeCurrent),
    (Recv::Window, "backend_name", Native::WindowHandleBackendName),
    (Recv::Option_, "is_some", Native::OptIsSome),
    (Recv::Option_, "is_none", Native::OptIsNone),
    (Recv::Option_, "unwrap", Native::OptUnwrap),
    (Recv::Option_, "unwrap_or", Native::OptUnwrapOr),
    (Recv::Option_, "map", Native::OptMap),
    (Recv::Option_, "and_then", Native::OptAndThen),
    (Recv::Option_, "or", Native::OptOr),
    (Recv::Result_, "is_ok", Native::ResIsOk),
    (Recv::Result_, "is_err", Native::ResIsErr),
    (Recv::Result_, "unwrap", Native::ResUnwrap),
    (Recv::Result_, "unwrap_or", Native::ResUnwrapOr),
    (Recv::Result_, "unwrap_err", Native::ResUnwrapErr),
    (Recv::Result_, "map", Native::ResMap),
    (Recv::Result_, "map_err", Native::ResMapErr),
    (Recv::Result_, "and_then", Native::ResAndThen),
    (Recv::Range, "to_list", Native::RangeToList),
    (Recv::Range, "contains", Native::RangeContains),
    (Recv::Range, "len", Native::RangeLen),
    (Recv::Range, "map", Native::RangeMap),
    (Recv::Range, "filter", Native::RangeFilter),
    (Recv::Range, "each", Native::RangeEach),
    (Recv::Range, "fold", Native::RangeFold),
    (Recv::Range, "rev", Native::RangeRev),
    (Recv::Range, "any", Native::RangeAny),
    (Recv::Range, "all", Native::RangeAll),
];

#[cfg(test)]
mod namespace_tests {
    use super::*;

    #[test]
    fn listed_namespace_members_resolve() {
        for ns in ["math", "fs", "os", "fft", "worker", "gpu", "window", "gfx"] {
            for name in Native::namespace_members(ns) {
                assert!(
                    Native::namespace_member(ns, name).is_some(),
                    "{ns}.{name} listed but does not resolve"
                );
            }
        }
    }
}
