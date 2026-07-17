//! Native function implementations.
//!
//! Calling convention: the arguments (for methods, the receiver first) are the
//! top `argc` stack values — they stay on the stack (rooted) while the native
//! runs; `finish_native` pops them and pushes the result. Values held only in
//! Rust locals across an allocation are pushed onto `vm.temp_roots` first.

use std::cmp::Ordering;
use std::io::Write;

use crate::builtins::Native;
use crate::types::{OPTION_DEF, OPTION_NONE, OPTION_SOME, RESULT_DEF, RESULT_ERR, RESULT_OK};
use crate::value::{FMap, Handle, Obj, Value};
use crate::vm::{fmt_float, Vm, VmError};

pub fn call_native(vm: &mut Vm, n: Native, argc: u8) -> Result<(), VmError> {
    use Native::*;
    let result: Value = match n {
        // ------------------------------------------------------------------
        // Free functions
        // ------------------------------------------------------------------
        Print | Println => {
            let s = vm.display_value(vm.native_arg(argc, 0))?;
            let nl = matches!(n, Println);
            let r = if nl {
                writeln!(vm.out, "{s}")
            } else {
                write!(vm.out, "{s}")
            };
            let _ = r;
            let _ = vm.out.flush();
            Value::Unit
        }
        Str => {
            let s = vm.display_value(vm.native_arg(argc, 0))?;
            vm.alloc_str(s)
        }
        Panic => {
            let msg = vm.str_of(vm.native_arg(argc, 0))?;
            return Err(vm.error(msg));
        }
        Assert => {
            let Value::Bool(ok) = vm.native_arg(argc, 0) else {
                return Err(vm.error("internal: assert expects Bool (VM bug)"));
            };
            if !ok {
                return Err(vm.error("assertion failed"));
            }
            Value::Unit
        }
        AssertEq => {
            let a = vm.native_arg(argc, 0);
            let b = vm.native_arg(argc, 1);
            let eq = vm.value_eq(a, b, 0).map_err(|m| vm.error(m))?;
            if !eq {
                let sa = vm.display_value(a)?;
                let sb = vm.display_value(b)?;
                return Err(vm.error(format!(
                    "assertion failed: values differ\n  left:  {sa}\n  right: {sb}"
                )));
            }
            Value::Unit
        }
        Clock => Value::Float(vm.elapsed_secs()),
        Input => {
            let mut line = String::new();
            match std::io::stdin().read_line(&mut line) {
                Ok(0) | Err(_) => make_none(vm),
                Ok(_) => {
                    while line.ends_with('\n') || line.ends_with('\r') {
                        line.pop();
                    }
                    let s = vm.alloc_str(line);
                    make_some(vm, s)
                }
            }
        }

        // ------------------------------------------------------------------
        // math.*
        // ------------------------------------------------------------------
        MathSqrt => Value::Float(float_arg(vm, argc, 0)?.sqrt()),
        MathSin => Value::Float(float_arg(vm, argc, 0)?.sin()),
        MathCos => Value::Float(float_arg(vm, argc, 0)?.cos()),
        MathTan => Value::Float(float_arg(vm, argc, 0)?.tan()),
        MathAtan => Value::Float(float_arg(vm, argc, 0)?.atan()),
        MathAtan2 => Value::Float(float_arg(vm, argc, 0)?.atan2(float_arg(vm, argc, 1)?)),
        MathLog => Value::Float(float_arg(vm, argc, 0)?.ln()),
        MathLog2 => Value::Float(float_arg(vm, argc, 0)?.log2()),
        MathExp => Value::Float(float_arg(vm, argc, 0)?.exp()),
        MathPow => Value::Float(float_arg(vm, argc, 0)?.powf(float_arg(vm, argc, 1)?)),
        MathFloor => Value::Float(float_arg(vm, argc, 0)?.floor()),
        MathCeil => Value::Float(float_arg(vm, argc, 0)?.ceil()),
        MathRound => Value::Float(float_arg(vm, argc, 0)?.round()),
        MathAbs => Value::Float(float_arg(vm, argc, 0)?.abs()),
        MathAbsInt => Value::Int(checked_abs(vm, int_arg(vm, argc, 0)?)?),
        MathMin => Value::Int(int_arg(vm, argc, 0)?.min(int_arg(vm, argc, 1)?)),
        MathMax => Value::Int(int_arg(vm, argc, 0)?.max(int_arg(vm, argc, 1)?)),
        MathMinFloat => Value::Float(float_arg(vm, argc, 0)?.min(float_arg(vm, argc, 1)?)),
        MathMaxFloat => Value::Float(float_arg(vm, argc, 0)?.max(float_arg(vm, argc, 1)?)),
        MathRandom => Value::Float(vm.rng_next()),
        MathSeed => {
            let s = int_arg(vm, argc, 0)?;
            vm.rng_seed(s);
            Value::Unit
        }
        MathRandInt => {
            let lo = int_arg(vm, argc, 0)?;
            let hi = int_arg(vm, argc, 1)?;
            if lo > hi {
                return Err(vm.error(format!("math.rand_int: empty range {lo}..={hi}")));
            }
            Value::Int(vm.rng_range(lo, hi))
        }
        MathLog10 => Value::Float(float_arg(vm, argc, 0)?.log10()),
        MathFmod => Value::Float(float_arg(vm, argc, 0)? % float_arg(vm, argc, 1)?),
        CharFromCode => {
            let code = int_arg(vm, argc, 0)?;
            let c = u32::try_from(code).ok().and_then(char::from_u32);
            match c {
                Some(c) => vm.char_str(c),
                None => {
                    return Err(vm.error(format!("char: invalid character code {code}")));
                }
            }
        }

        TryCall => {
            let f = vm.native_arg(argc, 0);
            match vm.call_value_caught(f) {
                Ok(v) => make_ok(vm, v),
                Err(msg) => {
                    let m = vm.alloc_str(msg);
                    make_err(vm, m)
                }
            }
        }

        // ------------------------------------------------------------------
        // fs.* / os.* (v0.3)
        // ------------------------------------------------------------------


        // ------------------------------------------------------------------
        // fft.* (v0.7)
        // ------------------------------------------------------------------
        FftFft | FftIfft => {
            let re = float_vec_arg(vm, argc, 0)?;
            let im = float_vec_arg(vm, argc, 1)?;
            if re.len() != im.len() {
                return Err(vm.error(format!(
                    "fft: re and im lengths differ ({} vs {})",
                    re.len(),
                    im.len()
                )));
            }
            if re.is_empty() {
                return Err(vm.error("fft: empty input"));
            }
            let (or_, oi) = if matches!(n, FftFft) {
                crate::fft::fft(&re, &im)
            } else {
                crate::fft::ifft(&re, &im)
            };
            floats_pair(vm, or_, oi)
        }
        FftRfft => {
            let x = float_vec_arg(vm, argc, 0)?;
            if x.is_empty() {
                return Err(vm.error("fft.rfft: empty input"));
            }
            let (or_, oi) = crate::fft::rfft(&x);
            floats_pair(vm, or_, oi)
        }
        FftMagnitude => {
            // Every rfft consumer wrote this same zip/hypot line; Floats
            // aren't heap objects, so building the result list needs no
            // extra GC rooting beyond `make_list`'s own allocation.
            let re = float_vec_arg(vm, argc, 0)?;
            let im = float_vec_arg(vm, argc, 1)?;
            if re.len() != im.len() {
                return Err(vm.error(format!(
                    "fft.magnitude: re and im lengths differ ({} vs {})",
                    re.len(),
                    im.len()
                )));
            }
            let mags = re.iter().zip(&im).map(|(r, i)| Value::Float(r.hypot(*i))).collect();
            make_list(vm, mags)
        }

        // ------------------------------------------------------------------
        // worker.* (v0.7) — OS-thread isolates, string channels
        // ------------------------------------------------------------------
        WorkerSpawn => {
            let file = str_arg(vm, argc, 0)?;
            let raw = list_arg(vm, argc, 1)?;
            let mut args = Vec::with_capacity(raw.len());
            for v in raw {
                args.push(vm.str_of(v)?);
            }
            let sink = vm
                .worker_sink
                .clone()
                .unwrap_or_else(crate::worker::stdout_sink);
            // Resolve relative to the entry script's directory, the same
            // rule imports use (absolute paths pass through untouched).
            // vm.entry_dir is set by every runner; None (REPL, string
            // sources) falls back to the working directory.
            let base = vm.entry_dir.clone().unwrap_or_default();
            match crate::worker::spawn(&file, args, &base, sink) {
                Ok(handle) => {
                    let h = vm.heap.alloc(Obj::Worker(std::rc::Rc::new(
                        std::cell::RefCell::new(handle),
                    )));
                    make_ok(vm, Value::Obj(h))
                }
                Err(msg) => {
                    let m = vm.alloc_str(msg);
                    make_err(vm, m)
                }
            }
        }
        WorkerHandleSend => {
            let w = worker_rc(vm, argc)?;
            let msg = str_arg(vm, argc, 1)?;
            let delivered = w.borrow().send(msg);
            Value::Bool(delivered)
        }
        WorkerHandleRecv => {
            let w = worker_rc(vm, argc)?;
            let got = w.borrow().recv(); // blocks until a message or hangup
            match got {
                Some(s) => {
                    let v = vm.alloc_str(s);
                    make_some(vm, v)
                }
                None => make_none(vm),
            }
        }
        WorkerHandleTryRecv => {
            let w = worker_rc(vm, argc)?;
            let got = w.borrow().try_recv(); // never blocks
            match got {
                None => make_none(vm), // no message ready right now
                Some(None) => {
                    let inner = make_none(vm);
                    make_some(vm, inner) // the worker finished
                }
                Some(Some(s)) => {
                    let v = vm.alloc_str(s);
                    let inner = make_some(vm, v);
                    make_some(vm, inner)
                }
            }
        }
        WorkerHandleJoin => {
            let w = worker_rc(vm, argc)?;
            let outcome = w.borrow_mut().join(); // blocks; idempotent
            match outcome {
                Ok(()) => make_ok(vm, Value::Unit),
                Err(msg) => {
                    let m = vm.alloc_str(msg);
                    make_err(vm, m)
                }
            }
        }
        WorkerSelfSend => {
            let msg = str_arg(vm, argc, 0)?;
            match &vm.worker_ctx {
                Some(ctx) => Value::Bool(ctx.tx.send(msg).is_ok()),
                None => return Err(vm.error("worker.send: not inside a worker")),
            }
        }
        WorkerSelfRecv => {
            let got = match &vm.worker_ctx {
                Some(ctx) => ctx.rx.recv().ok(), // blocks until a message or hangup
                None => return Err(vm.error("worker.recv: not inside a worker")),
            };
            match got {
                Some(s) => {
                    let v = vm.alloc_str(s);
                    make_some(vm, v)
                }
                None => make_none(vm),
            }
        }
        WorkerSelfTryRecv => {
            use std::sync::mpsc::TryRecvError;
            let got = match &vm.worker_ctx {
                Some(ctx) => match ctx.rx.try_recv() {
                    Ok(s) => Some(Some(s)),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => Some(None),
                },
                None => return Err(vm.error("worker.try_recv: not inside a worker")),
            };
            match got {
                None => make_none(vm), // no message ready right now
                Some(None) => {
                    let inner = make_none(vm);
                    make_some(vm, inner) // the parent hung up
                }
                Some(Some(s)) => {
                    let v = vm.alloc_str(s);
                    let inner = make_some(vm, v);
                    make_some(vm, inner)
                }
            }
        }
        WorkerIsWorker => Value::Bool(vm.worker_ctx.is_some()),

        // ------------------------------------------------------------------
        // Bytes (v0.7)
        // ------------------------------------------------------------------
        BytesNew => {
            let n = int_arg(vm, argc, 0)?;
            if n < 0 {
                return Err(vm.error(format!("bytes: negative length {n}")));
            }
            Value::Obj(vm.heap.alloc(Obj::Bytes(vec![0u8; n as usize])))
        }
        BytesOf => {
            let out = {
                let items = list_ref(vm, argc)?;
                let mut out = Vec::with_capacity(items.len());
                for &v in items {
                    match v {
                        Value::Int(b) if (0..=255).contains(&b) => out.push(b as u8),
                        Value::Int(b) => {
                            return Err(vm.error(format!(
                                "bytes_of: value {b} is not a byte (0..255)"
                            )));
                        }
                        _ => return Err(vm.error("bytes_of: list must contain Ints")),
                    }
                }
                out
            };
            Value::Obj(vm.heap.alloc(Obj::Bytes(out)))
        }
        BytesLen => Value::Int(bytes_ref(vm, argc)?.len() as i64),
        BytesGet => {
            let i = int_arg(vm, argc, 1)?;
            let bs = bytes_ref(vm, argc)?;
            if i < 0 || i as usize >= bs.len() {
                return Err(vm.error(format!(
                    "bytes index out of bounds: index {i}, length {}",
                    bs.len()
                )));
            }
            Value::Int(bs[i as usize] as i64)
        }
        BytesSet => {
            let i = int_arg(vm, argc, 1)?;
            let v = int_arg(vm, argc, 2)?;
            if !(0..=255).contains(&v) {
                return Err(vm.error(format!("bytes set: value {v} is not a byte (0..255)")));
            }
            let h = bytes_handle(vm, argc)?;
            let len = bytes_ref(vm, argc)?.len();
            if i < 0 || i as usize >= len {
                return Err(vm.error(format!(
                    "bytes index out of bounds: index {i}, length {len}"
                )));
            }
            if let Obj::Bytes(bs) = vm.heap.get_mut(h) {
                bs[i as usize] = v as u8;
            }
            Value::Unit
        }
        BytesPush => {
            let v = int_arg(vm, argc, 1)?;
            if !(0..=255).contains(&v) {
                return Err(vm.error(format!("bytes push: value {v} is not a byte (0..255)")));
            }
            let h = bytes_handle(vm, argc)?;
            if let Obj::Bytes(bs) = vm.heap.get_mut(h) {
                bs.push(v as u8);
            }
            Value::Unit
        }
        BytesPushU16le => {
            let v = int_arg(vm, argc, 1)?;
            if !(0..=65535).contains(&v) {
                return Err(vm.error(format!("push_u16le: value {v} out of range 0..65535")));
            }
            let h = bytes_handle(vm, argc)?;
            if let Obj::Bytes(bs) = vm.heap.get_mut(h) {
                bs.extend_from_slice(&(v as u16).to_le_bytes());
            }
            Value::Unit
        }
        BytesPushI16le => {
            let v = int_arg(vm, argc, 1)?;
            if !(-32768..=32767).contains(&v) {
                return Err(vm.error(format!(
                    "push_i16le: value {v} out of range -32768..32767"
                )));
            }
            let h = bytes_handle(vm, argc)?;
            if let Obj::Bytes(bs) = vm.heap.get_mut(h) {
                bs.extend_from_slice(&(v as i16).to_le_bytes());
            }
            Value::Unit
        }
        BytesPushU32le => {
            let v = int_arg(vm, argc, 1)?;
            if !(0..=4294967295).contains(&v) {
                return Err(vm.error(format!(
                    "push_u32le: value {v} out of range 0..4294967295"
                )));
            }
            let h = bytes_handle(vm, argc)?;
            if let Obj::Bytes(bs) = vm.heap.get_mut(h) {
                bs.extend_from_slice(&(v as u32).to_le_bytes());
            }
            Value::Unit
        }
        BytesPushBytes => {
            // Snapshot the argument first: `b.push_bytes(b)` must append
            // b's old contents, not loop over a buffer growing under it.
            let other = bytes_arg(vm, argc, 1)?.clone();
            let h = bytes_handle(vm, argc)?;
            if let Obj::Bytes(bs) = vm.heap.get_mut(h) {
                bs.extend_from_slice(&other);
            }
            Value::Unit
        }
        BytesPushStr => {
            let s = str_arg(vm, argc, 1)?;
            let h = bytes_handle(vm, argc)?;
            if let Obj::Bytes(bs) = vm.heap.get_mut(h) {
                bs.extend_from_slice(s.as_bytes());
            }
            Value::Unit
        }
        BytesPushU16be => {
            let v = int_arg(vm, argc, 1)?;
            if !(0..=65535).contains(&v) {
                return Err(vm.error(format!("push_u16be: value {v} out of range 0..65535")));
            }
            let h = bytes_handle(vm, argc)?;
            if let Obj::Bytes(bs) = vm.heap.get_mut(h) {
                bs.extend_from_slice(&(v as u16).to_be_bytes());
            }
            Value::Unit
        }
        BytesPushU32be => {
            let v = int_arg(vm, argc, 1)?;
            if !(0..=4294967295).contains(&v) {
                return Err(vm.error(format!(
                    "push_u32be: value {v} out of range 0..4294967295"
                )));
            }
            let h = bytes_handle(vm, argc)?;
            if let Obj::Bytes(bs) = vm.heap.get_mut(h) {
                bs.extend_from_slice(&(v as u32).to_be_bytes());
            }
            Value::Unit
        }
        BytesPushU64le => {
            // No range check: `Int` is already the 64-bit two's-complement
            // value, so its own bit pattern is exactly what gets written.
            let v = int_arg(vm, argc, 1)?;
            let h = bytes_handle(vm, argc)?;
            if let Obj::Bytes(bs) = vm.heap.get_mut(h) {
                bs.extend_from_slice(&v.to_le_bytes());
            }
            Value::Unit
        }
        BytesPushU64be => {
            let v = int_arg(vm, argc, 1)?;
            let h = bytes_handle(vm, argc)?;
            if let Obj::Bytes(bs) = vm.heap.get_mut(h) {
                bs.extend_from_slice(&v.to_be_bytes());
            }
            Value::Unit
        }
        BytesPushF32le => {
            let v = float_arg(vm, argc, 1)?;
            let h = bytes_handle(vm, argc)?;
            if let Obj::Bytes(bs) = vm.heap.get_mut(h) {
                bs.extend_from_slice(&(v as f32).to_le_bytes());
            }
            Value::Unit
        }
        BytesPushF32be => {
            let v = float_arg(vm, argc, 1)?;
            let h = bytes_handle(vm, argc)?;
            if let Obj::Bytes(bs) = vm.heap.get_mut(h) {
                bs.extend_from_slice(&(v as f32).to_be_bytes());
            }
            Value::Unit
        }
        BytesReadF32le | BytesReadF32be => {
            let off = int_arg(vm, argc, 1)?;
            let bs = bytes_ref(vm, argc)?;
            let len = bs.len() as i64;
            if off < 0 || off > len - 4 {
                let i = if off < 0 { off } else { off.max(len) };
                return Err(vm.error(format!(
                    "bytes index out of bounds: index {i}, length {len}"
                )));
            }
            let o = off as usize;
            let bytes4 = [bs[o], bs[o + 1], bs[o + 2], bs[o + 3]];
            let v = if n == BytesReadF32le {
                f32::from_le_bytes(bytes4)
            } else {
                f32::from_be_bytes(bytes4)
            };
            Value::Float(v as f64)
        }
        BytesReadU16le | BytesReadI16le | BytesReadU16be | BytesReadU32le | BytesReadU32be
        | BytesReadU64le | BytesReadU64be => {
            let off = int_arg(vm, argc, 1)?;
            let width = match n {
                BytesReadU64le | BytesReadU64be => 8i64,
                BytesReadU32le | BytesReadU32be => 4i64,
                _ => 2i64,
            };
            let bs = bytes_ref(vm, argc)?;
            let len = bs.len() as i64;
            if off < 0 || off > len - width {
                // Panic naming the first out-of-range byte the read would
                // touch, so the message matches a plain `get` of that index.
                let i = if off < 0 { off } else { off.max(len) };
                return Err(vm.error(format!(
                    "bytes index out of bounds: index {i}, length {len}"
                )));
            }
            let o = off as usize;
            let v = match n {
                BytesReadU16le => u16::from_le_bytes([bs[o], bs[o + 1]]) as i64,
                BytesReadI16le => i16::from_le_bytes([bs[o], bs[o + 1]]) as i64,
                BytesReadU16be => u16::from_be_bytes([bs[o], bs[o + 1]]) as i64,
                BytesReadU32le => {
                    u32::from_le_bytes([bs[o], bs[o + 1], bs[o + 2], bs[o + 3]]) as i64
                }
                BytesReadU32be => {
                    u32::from_be_bytes([bs[o], bs[o + 1], bs[o + 2], bs[o + 3]]) as i64
                }
                BytesReadU64le => i64::from_le_bytes([
                    bs[o], bs[o + 1], bs[o + 2], bs[o + 3], bs[o + 4], bs[o + 5], bs[o + 6],
                    bs[o + 7],
                ]),
                _ => i64::from_be_bytes([
                    bs[o], bs[o + 1], bs[o + 2], bs[o + 3], bs[o + 4], bs[o + 5], bs[o + 6],
                    bs[o + 7],
                ]),
            };
            Value::Int(v)
        }
        BytesSlice => {
            // Clamped copy, like List.slice: start inclusive, end exclusive.
            let a = int_arg(vm, argc, 1)?;
            let b = int_arg(vm, argc, 2)?;
            let bs = bytes_ref(vm, argc)?;
            let len = bs.len() as i64;
            let start = a.clamp(0, len) as usize;
            let end = b.clamp(0, len) as usize;
            let out = if start >= end { Vec::new() } else { bs[start..end].to_vec() };
            Value::Obj(vm.heap.alloc(Obj::Bytes(out)))
        }
        BytesConcat => {
            let out = {
                let other = match vm.native_arg(argc, 1) {
                    Value::Obj(h) => match vm.heap.get(h) {
                        Obj::Bytes(bs) => bs,
                        _ => return Err(vm.error("concat: expected Bytes")),
                    },
                    _ => return Err(vm.error("concat: expected Bytes")),
                };
                let a = bytes_ref(vm, argc)?;
                let mut out = Vec::with_capacity(a.len() + other.len());
                out.extend_from_slice(a);
                out.extend_from_slice(other);
                out
            };
            Value::Obj(vm.heap.alloc(Obj::Bytes(out)))
        }
        BytesToList => {
            let items: Vec<Value> =
                bytes_ref(vm, argc)?.iter().map(|&b| Value::Int(b as i64)).collect();
            make_list_prerooted(vm, items) // all Ints: nothing to root
        }
        BytesUtf8 => {
            let bs = bytes_ref(vm, argc)?.clone();
            match String::from_utf8(bs) {
                Ok(text) => {
                    let v = vm.alloc_str(text);
                    make_ok(vm, v)
                }
                Err(e) => {
                    let msg = vm.alloc_str(format!("invalid UTF-8: {e}"));
                    make_err(vm, msg)
                }
            }
        }
        StrToBytes => {
            let s = recv_str(vm, argc)?;
            Value::Obj(vm.heap.alloc(Obj::Bytes(s.into_bytes())))
        }
        FsReadBytes => {
            let path = str_arg(vm, argc, 0)?;
            match std::fs::read(&path) {
                Ok(data) => {
                    let v = Value::Obj(vm.heap.alloc(Obj::Bytes(data)));
                    make_ok(vm, v)
                }
                Err(e) => make_io_err(vm, &path, e),
            }
        }
        FsWriteBytes => {
            let path = str_arg(vm, argc, 0)?;
            let data = match vm.native_arg(argc, 1) {
                Value::Obj(h) => match vm.heap.get(h) {
                    Obj::Bytes(bs) => bs.clone(),
                    _ => return Err(vm.error("fs.write_bytes: expected Bytes")),
                },
                _ => return Err(vm.error("fs.write_bytes: expected Bytes")),
            };
            match std::fs::write(&path, data) {
                Ok(()) => make_ok(vm, Value::Unit),
                Err(e) => make_io_err(vm, &path, e),
            }
        }

        FsRead => {
            let path = str_arg(vm, argc, 0)?;
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    let v = vm.alloc_str(text);
                    make_ok(vm, v)
                }
                Err(e) => make_io_err(vm, &path, e),
            }
        }
        FsWrite => {
            let path = str_arg(vm, argc, 0)?;
            let contents = str_arg(vm, argc, 1)?;
            match std::fs::write(&path, contents) {
                Ok(()) => make_ok(vm, Value::Unit),
                Err(e) => make_io_err(vm, &path, e),
            }
        }
        FsAppend => {
            let path = str_arg(vm, argc, 0)?;
            let contents = str_arg(vm, argc, 1)?;
            let r = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .and_then(|mut f| std::io::Write::write_all(&mut f, contents.as_bytes()));
            match r {
                Ok(()) => make_ok(vm, Value::Unit),
                Err(e) => make_io_err(vm, &path, e),
            }
        }
        FsExists => {
            let path = str_arg(vm, argc, 0)?;
            Value::Bool(std::path::Path::new(&path).exists())
        }
        FsIsDir => {
            let path = str_arg(vm, argc, 0)?;
            Value::Bool(std::path::Path::new(&path).is_dir())
        }
        FsListDir => {
            let path = str_arg(vm, argc, 0)?;
            match std::fs::read_dir(&path) {
                Ok(entries) => {
                    let mut names: Vec<String> = entries
                        .filter_map(|e| e.ok())
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect();
                    names.sort();
                    let out = alloc_rooted_list(vm, Vec::new());
                    for n in names {
                        let nv = vm.alloc_str(n);
                        push_into(vm, out, nv);
                    }
                    let list = finish_rooted(vm, 1, Value::Obj(out));
                    make_ok(vm, list)
                }
                Err(e) => make_io_err(vm, &path, e),
            }
        }
        FsCreateDir => {
            let path = str_arg(vm, argc, 0)?;
            match std::fs::create_dir_all(&path) {
                Ok(()) => make_ok(vm, Value::Unit),
                Err(e) => make_io_err(vm, &path, e),
            }
        }
        FsRemove => {
            let path = str_arg(vm, argc, 0)?;
            let p = std::path::Path::new(&path);
            let r = if p.is_dir() { std::fs::remove_dir(p) } else { std::fs::remove_file(p) };
            match r {
                Ok(()) => make_ok(vm, Value::Unit),
                Err(e) => make_io_err(vm, &path, e),
            }
        }
        OsArgs => {
            let args = vm.script_args.clone();
            let out = alloc_rooted_list(vm, Vec::new());
            for a in args {
                let av = vm.alloc_str(a);
                push_into(vm, out, av);
            }
            finish_rooted(vm, 1, Value::Obj(out))
        }
        OsEnv => {
            let name = str_arg(vm, argc, 0)?;
            match std::env::var(&name) {
                Ok(v) => {
                    let sv = vm.alloc_str(v);
                    make_some(vm, sv)
                }
                Err(_) => make_none(vm),
            }
        }
        OsRun => {
            let cmd = str_arg(vm, argc, 0)?;
            let arg_list = list_arg(vm, argc, 1)?;
            let mut cmd_args = Vec::with_capacity(arg_list.len());
            for v in arg_list {
                cmd_args.push(vm.str_of(v)?);
            }
            match std::process::Command::new(&cmd).args(&cmd_args).output() {
                Ok(out) => {
                    let code = Value::Int(i64::from(out.status.code().unwrap_or(-1)));
                    let stdout = vm.alloc_str(String::from_utf8_lossy(&out.stdout).into_owned());
                    vm.temp_roots.push(stdout);
                    let stderr = vm.alloc_str(String::from_utf8_lossy(&out.stderr).into_owned());
                    vm.temp_roots.push(stderr);
                    let t = make_tuple(vm, vec![code, stdout, stderr]);
                    vm.temp_roots.truncate(vm.temp_roots.len() - 2);
                    make_ok(vm, t)
                }
                Err(e) => {
                    let msg = vm.alloc_str(format!("cannot run `{cmd}`: {e}"));
                    make_err(vm, msg)
                }
            }
        }
        OsExit => {
            let code = int_arg(vm, argc, 0)?;
            std::process::exit(code as i32);
        }
        OsTime => {
            use std::time::{SystemTime, UNIX_EPOCH};
            let secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
            Value::Float(secs)
        }

        // ------------------------------------------------------------------
        // gpu.* (v0.7, experimental) — implementations live in src/gpu.rs;
        // without the `gpu` cargo feature they degrade gracefully.
        // ------------------------------------------------------------------
        GpuAvailable => Value::Bool(crate::gpu::available()),
        GpuAdapterInfo => vm.alloc_str(crate::gpu::adapter_info()),
        GpuRunSpirv => {
            let spirv = bytes_arg(vm, argc, 0)?.clone();
            let input = bytes_arg(vm, argc, 1)?.clone();
            let out_len = int_arg(vm, argc, 2)?;
            let wx = int_arg(vm, argc, 3)?;
            let wy = int_arg(vm, argc, 4)?;
            let wz = int_arg(vm, argc, 5)?;
            // Argument-domain failures are Err values (the whole API returns
            // Result), never panics -- mirrors GpuRun exactly.
            let outcome = match (
                usize::try_from(out_len),
                u32::try_from(wx),
                u32::try_from(wy),
                u32::try_from(wz),
            ) {
                (Ok(out_len), Ok(wx), Ok(wy), Ok(wz)) => {
                    crate::gpu::run_spirv(&spirv, &input, out_len, wx, wy, wz)
                }
                (Err(_), ..) => {
                    Err(format!("gpu.run_spirv: out_len must be positive, got {out_len}"))
                }
                _ => Err(format!(
                    "gpu.run_spirv: workgroup counts must be positive, got ({wx}, {wy}, {wz})"
                )),
            };
            match outcome {
                Ok(data) => {
                    let b = Value::Obj(vm.alloc(Obj::Bytes(data)));
                    make_ok(vm, b)
                }
                Err(msg) => {
                    let m = vm.alloc_str(msg);
                    make_err(vm, m)
                }
            }
        }
        GpuBackend => vm.alloc_str(crate::gpu::backend().to_string()),
        GpuRun => {
            let wgsl = str_arg(vm, argc, 0)?;
            let input = bytes_arg(vm, argc, 1)?.clone();
            let out_len = int_arg(vm, argc, 2)?;
            let wx = int_arg(vm, argc, 3)?;
            let wy = int_arg(vm, argc, 4)?;
            let wz = int_arg(vm, argc, 5)?;
            // Argument-domain failures are Err values (the whole API returns
            // Result), never panics.
            let outcome = match (
                usize::try_from(out_len),
                u32::try_from(wx),
                u32::try_from(wy),
                u32::try_from(wz),
            ) {
                (Ok(out_len), Ok(wx), Ok(wy), Ok(wz)) => {
                    crate::gpu::run(&wgsl, &input, out_len, wx, wy, wz)
                }
                (Err(_), ..) => Err(format!("gpu.run: out_len must be positive, got {out_len}")),
                _ => Err(format!(
                    "gpu.run: workgroup counts must be positive, got ({wx}, {wy}, {wz})"
                )),
            };
            match outcome {
                Ok(data) => {
                    let b = Value::Obj(vm.alloc(Obj::Bytes(data)));
                    make_ok(vm, b)
                }
                Err(msg) => {
                    let m = vm.alloc_str(msg);
                    make_err(vm, m)
                }
            }
        }

        // ------------------------------------------------------------------
        // window.* (v0.8: Linux/Windows/macOS via `gl`; v0.9 adds a
        // Metal-backed `create_metal` sibling on macOS, additive alongside
        // `gl`, never a replacement; Linux then gains a Vulkan-backed
        // `create_vulkan` sibling the same way) — implementation lives in
        // src/window/; without the relevant cargo feature each entry point
        // degrades gracefully (see src/window/mod.rs).
        // ------------------------------------------------------------------
        WindowCreate => {
            let title = str_arg(vm, argc, 0)?;
            let w = int_arg(vm, argc, 1)?;
            let h = int_arg(vm, argc, 2)?;
            match crate::window::create(&title, w as i32, h as i32) {
                Ok(handle) => {
                    let h = vm.heap.alloc(Obj::Window(std::rc::Rc::new(
                        std::cell::RefCell::new(handle),
                    )));
                    make_ok(vm, Value::Obj(h))
                }
                Err(msg) => {
                    let m = vm.alloc_str(msg);
                    make_err(vm, m)
                }
            }
        }
        WindowCreateMetal => {
            let title = str_arg(vm, argc, 0)?;
            let w = int_arg(vm, argc, 1)?;
            let h = int_arg(vm, argc, 2)?;
            match crate::window::create_metal(&title, w as i32, h as i32) {
                Ok(handle) => {
                    let h = vm.heap.alloc(Obj::Window(std::rc::Rc::new(
                        std::cell::RefCell::new(handle),
                    )));
                    make_ok(vm, Value::Obj(h))
                }
                Err(msg) => {
                    let m = vm.alloc_str(msg);
                    make_err(vm, m)
                }
            }
        }
        WindowCreateVulkan => {
            let title = str_arg(vm, argc, 0)?;
            let w = int_arg(vm, argc, 1)?;
            let h = int_arg(vm, argc, 2)?;
            match crate::window::create_vulkan(&title, w as i32, h as i32) {
                Ok(handle) => {
                    let h = vm.heap.alloc(Obj::Window(std::rc::Rc::new(
                        std::cell::RefCell::new(handle),
                    )));
                    make_ok(vm, Value::Obj(h))
                }
                Err(msg) => {
                    let m = vm.alloc_str(msg);
                    make_err(vm, m)
                }
            }
        }
        WindowHandlePoll => {
            let w = window_rc(vm, argc)?;
            w.borrow_mut().poll();
            Value::Unit
        }
        WindowHandleShouldClose => {
            let w = window_rc(vm, argc)?;
            let b = w.borrow().should_close();
            Value::Bool(b)
        }
        WindowHandleClose => {
            let w = window_rc(vm, argc)?;
            w.borrow_mut().close();
            Value::Unit
        }
        WindowHandleKeyDown => {
            let w = window_rc(vm, argc)?;
            let name = str_arg(vm, argc, 1)?;
            let b = w.borrow().key_down(&name);
            Value::Bool(b)
        }
        WindowHandleMousePos => {
            let w = window_rc(vm, argc)?;
            let (x, y) = w.borrow().mouse_pos();
            make_tuple(vm, vec![Value::Float(x), Value::Float(y)])
        }
        WindowHandleWidth => {
            let w = window_rc(vm, argc)?;
            let width = w.borrow().width();
            Value::Int(i64::from(width))
        }
        WindowHandleHeight => {
            let w = window_rc(vm, argc)?;
            let height = w.borrow().height();
            Value::Int(i64::from(height))
        }
        WindowHandleClear => {
            let w = window_rc(vm, argc)?;
            let r = float_arg(vm, argc, 1)?;
            let g = float_arg(vm, argc, 2)?;
            let b = float_arg(vm, argc, 3)?;
            let a = float_arg(vm, argc, 4)?;
            w.borrow_mut().clear(r, g, b, a);
            Value::Unit
        }
        WindowHandleSwapBuffers => {
            let w = window_rc(vm, argc)?;
            w.borrow_mut().swap_buffers();
            Value::Unit
        }
        WindowHandleMakeCurrent => {
            let w = window_rc(vm, argc)?;
            w.borrow_mut().make_current();
            vm.gfx_current_window = Some(w);
            Value::Unit
        }
        WindowHandleBackendName => {
            let w = window_rc(vm, argc)?;
            let name = w.borrow().backend_name();
            vm.alloc_str(name)
        }

        // ------------------------------------------------------------------
        // gfx.* (v0.8) — OpenGL draw calls against "whichever window is
        // currently current" (see `Vm::gfx_current_window`, set by
        // `win.make_current()` above). Implementation lives in
        // `src/window/`; without the `gl` cargo feature, or before any
        // window has ever called `make_current()`, every member panics
        // with a clear message (`gfx_window`, below) — except
        // `compile_program`, whose failures are `Err` values instead,
        // matching its `Result` return (the only fallible `gfx.*` member).
        // ------------------------------------------------------------------
        GfxCompileProgram => {
            let vsrc = str_arg(vm, argc, 0)?;
            let fsrc = str_arg(vm, argc, 1)?;
            match gfx_window_msg(vm) {
                Ok(w) => {
                    let compiled = w.borrow_mut().gl_compile_program(&vsrc, &fsrc);
                    match compiled {
                        Ok(p) => make_ok(vm, Value::Int(p as i64)),
                        Err(msg) => {
                            let m = vm.alloc_str(msg);
                            make_err(vm, m)
                        }
                    }
                }
                Err(msg) => {
                    let m = vm.alloc_str(msg);
                    make_err(vm, m)
                }
            }
        }
        GfxUseProgram => {
            let w = gfx_window(vm)?;
            let p = u32_arg(vm, argc, 0)?;
            w.borrow_mut().gl_use_program(p);
            Value::Unit
        }
        GfxDeleteProgram => {
            let w = gfx_window(vm)?;
            let p = u32_arg(vm, argc, 0)?;
            w.borrow_mut().gl_delete_program(p);
            Value::Unit
        }
        GfxCreateBuffer => {
            let w = gfx_window(vm)?;
            let name = w.borrow_mut().gl_create_buffer();
            Value::Int(name as i64)
        }
        GfxDeleteBuffer => {
            let w = gfx_window(vm)?;
            let b = u32_arg(vm, argc, 0)?;
            w.borrow_mut().gl_delete_buffer(b);
            Value::Unit
        }
        GfxBindBuffer => {
            let w = gfx_window(vm)?;
            let kind = buffer_kind_arg(vm, argc, 0)?;
            let b = u32_arg(vm, argc, 1)?;
            w.borrow_mut().gl_bind_buffer(kind, b);
            Value::Unit
        }
        GfxUploadBuffer => {
            let w = gfx_window(vm)?;
            let kind = buffer_kind_arg(vm, argc, 0)?;
            let data = bytes_arg(vm, argc, 1)?.clone();
            let dynamic = expect_bool(vm, vm.native_arg(argc, 2))?;
            w.borrow_mut().gl_upload_buffer(kind, &data, dynamic);
            Value::Unit
        }
        GfxCreateVertexArray => {
            let w = gfx_window(vm)?;
            let name = w.borrow_mut().gl_create_vertex_array();
            Value::Int(name as i64)
        }
        GfxBindVertexArray => {
            let w = gfx_window(vm)?;
            let v = u32_arg(vm, argc, 0)?;
            w.borrow_mut().gl_bind_vertex_array(v);
            Value::Unit
        }
        GfxDeleteVertexArray => {
            let w = gfx_window(vm)?;
            let v = u32_arg(vm, argc, 0)?;
            w.borrow_mut().gl_delete_vertex_array(v);
            Value::Unit
        }
        GfxSetVertexAttrib => {
            let w = gfx_window(vm)?;
            let index = u32_arg(vm, argc, 0)?;
            let size = i32_arg(vm, argc, 1)?;
            let stride = i32_arg(vm, argc, 2)?;
            let offset = i32_arg(vm, argc, 3)?;
            w.borrow_mut()
                .gl_set_vertex_attrib(index, size, stride, offset);
            Value::Unit
        }
        GfxDisableVertexAttrib => {
            let w = gfx_window(vm)?;
            let index = u32_arg(vm, argc, 0)?;
            w.borrow_mut().gl_disable_vertex_attrib(index);
            Value::Unit
        }
        GfxCreateTexture => {
            let w = gfx_window(vm)?;
            let name = w.borrow_mut().gl_create_texture();
            Value::Int(name as i64)
        }
        GfxDeleteTexture => {
            let w = gfx_window(vm)?;
            let t = u32_arg(vm, argc, 0)?;
            w.borrow_mut().gl_delete_texture(t);
            Value::Unit
        }
        GfxBindTexture => {
            let w = gfx_window(vm)?;
            let t = u32_arg(vm, argc, 0)?;
            w.borrow_mut().gl_bind_texture(t);
            Value::Unit
        }
        GfxActiveTextureUnit => {
            let w = gfx_window(vm)?;
            let unit = u32_arg(vm, argc, 0)?;
            w.borrow_mut().gl_active_texture_unit(unit);
            Value::Unit
        }
        GfxUploadTexture => {
            let w = gfx_window(vm)?;
            let data = bytes_arg(vm, argc, 0)?.clone();
            let width = i32_arg(vm, argc, 1)?;
            let height = i32_arg(vm, argc, 2)?;
            let has_alpha = expect_bool(vm, vm.native_arg(argc, 3))?;
            // `width`/`height` come straight from the caller and drive how
            // many bytes `glTexImage2D` reads from `data.as_ptr()` -- with
            // no check here, a too-short buffer is an out-of-bounds heap
            // read reaching the GL driver (demos/png/png.fable's own
            // `pixels.len() != w * h * 3` check is the same guard one
            // directory over; this native had none).
            let bpp: i64 = if has_alpha { 4 } else { 3 };
            let expected = (width as i64) * (height as i64) * bpp;
            if width < 0 || height < 0 || data.len() as i64 != expected {
                return Err(vm.error(format!(
                    "gfx.upload_texture: data has {} bytes, but {width}x{height}x{bpp} \
                     ({has_alpha}) requires exactly {expected}",
                    data.len()
                )));
            }
            w.borrow_mut()
                .gl_upload_texture(&data, width, height, has_alpha);
            Value::Unit
        }
        GfxSetUniformInt => {
            let w = gfx_window(vm)?;
            let p = u32_arg(vm, argc, 0)?;
            let name = str_arg(vm, argc, 1)?;
            let v = i32_arg(vm, argc, 2)?;
            w.borrow_mut().gl_set_uniform_int(p, &name, v);
            Value::Unit
        }
        GfxSetUniformFloat => {
            let w = gfx_window(vm)?;
            let p = u32_arg(vm, argc, 0)?;
            let name = str_arg(vm, argc, 1)?;
            let v = float_arg(vm, argc, 2)? as f32;
            w.borrow_mut().gl_set_uniform_float(p, &name, v);
            Value::Unit
        }
        GfxSetUniformVec2 => {
            let w = gfx_window(vm)?;
            let p = u32_arg(vm, argc, 0)?;
            let name = str_arg(vm, argc, 1)?;
            let x = float_arg(vm, argc, 2)? as f32;
            let y = float_arg(vm, argc, 3)? as f32;
            w.borrow_mut().gl_set_uniform_vec2(p, &name, x, y);
            Value::Unit
        }
        GfxSetUniformVec3 => {
            let w = gfx_window(vm)?;
            let p = u32_arg(vm, argc, 0)?;
            let name = str_arg(vm, argc, 1)?;
            let x = float_arg(vm, argc, 2)? as f32;
            let y = float_arg(vm, argc, 3)? as f32;
            let z = float_arg(vm, argc, 4)? as f32;
            w.borrow_mut().gl_set_uniform_vec3(p, &name, x, y, z);
            Value::Unit
        }
        GfxSetUniformVec4 => {
            let w = gfx_window(vm)?;
            let p = u32_arg(vm, argc, 0)?;
            let name = str_arg(vm, argc, 1)?;
            let x = float_arg(vm, argc, 2)? as f32;
            let y = float_arg(vm, argc, 3)? as f32;
            let z = float_arg(vm, argc, 4)? as f32;
            let w4 = float_arg(vm, argc, 5)? as f32;
            w.borrow_mut().gl_set_uniform_vec4(p, &name, x, y, z, w4);
            Value::Unit
        }
        GfxSetUniformMat4 => {
            let w = gfx_window(vm)?;
            let p = u32_arg(vm, argc, 0)?;
            let name = str_arg(vm, argc, 1)?;
            let values = mat4_arg(vm, argc, 2)?;
            w.borrow_mut().gl_set_uniform_mat4(p, &name, &values);
            Value::Unit
        }
        GfxDrawArrays => {
            let w = gfx_window(vm)?;
            let first = i32_arg(vm, argc, 0)?;
            let count = i32_arg(vm, argc, 1)?;
            w.borrow_mut().gl_draw_arrays(first, count);
            Value::Unit
        }
        GfxDrawElements => {
            let w = gfx_window(vm)?;
            let count = i32_arg(vm, argc, 0)?;
            let byte_offset = i32_arg(vm, argc, 1)?;
            w.borrow_mut().gl_draw_elements(count, byte_offset);
            Value::Unit
        }
        GfxClear => {
            let w = gfx_window(vm)?;
            let r = float_arg(vm, argc, 0)? as f32;
            let g = float_arg(vm, argc, 1)? as f32;
            let b = float_arg(vm, argc, 2)? as f32;
            let a = float_arg(vm, argc, 3)? as f32;
            w.borrow_mut().gl_clear(r, g, b, a);
            Value::Unit
        }
        GfxSetDepthTest => {
            let w = gfx_window(vm)?;
            let enabled = expect_bool(vm, vm.native_arg(argc, 0))?;
            w.borrow_mut().gl_set_depth_test(enabled);
            Value::Unit
        }
        GfxViewport => {
            let w = gfx_window(vm)?;
            let x = i32_arg(vm, argc, 0)?;
            let y = i32_arg(vm, argc, 1)?;
            let width = i32_arg(vm, argc, 2)?;
            let height = i32_arg(vm, argc, 3)?;
            w.borrow_mut().gl_viewport(x, y, width, height);
            Value::Unit
        }
        GfxReadPixels => {
            let w = gfx_window(vm)?;
            let x = i32_arg(vm, argc, 0)?;
            let y = i32_arg(vm, argc, 1)?;
            let width = i32_arg(vm, argc, 2)?;
            let height = i32_arg(vm, argc, 3)?;
            let data = w.borrow_mut().gl_read_pixels(x, y, width, height);
            Value::Obj(vm.heap.alloc(Obj::Bytes(data)))
        }

        // ------------------------------------------------------------------
        // List methods (receiver = arg 0)
        // ------------------------------------------------------------------
        ListLen => Value::Int(list_ref(vm, argc)?.len() as i64),
        ListIsEmpty => Value::Bool(list_ref(vm, argc)?.is_empty()),
        ListPush => {
            let v = vm.native_arg(argc, 1);
            with_list_mut(vm, argc, |items| items.push(v))?;
            Value::Unit
        }
        ListPop => {
            let popped = with_list_mut(vm, argc, |items| items.pop())?;
            match popped {
                Some(v) => make_some(vm, v),
                None => make_none(vm),
            }
        }
        ListInsert => {
            let idx = int_arg(vm, argc, 1)?;
            let v = vm.native_arg(argc, 2);
            let h = list_handle(vm, argc)?;
            let len = list_ref(vm, argc)?.len();
            if idx < 0 || idx as usize > len {
                return Err(vm.error(format!(
                    "insert index out of bounds: index {idx}, length {len}"
                )));
            }
            if let Obj::List(items) = vm.heap.get_mut(h) {
                items.insert(idx as usize, v);
            }
            Value::Unit
        }
        ListRemove => {
            let idx = int_arg(vm, argc, 1)?;
            let h = list_handle(vm, argc)?;
            let len = list_ref(vm, argc)?.len();
            if idx < 0 || idx as usize >= len {
                return Err(vm.error(format!(
                    "remove index out of bounds: index {idx}, length {len}"
                )));
            }
            match vm.heap.get_mut(h) {
                Obj::List(items) => items.remove(idx as usize),
                _ => Value::Unit,
            }
        }
        ListGet => {
            let idx = int_arg(vm, argc, 1)?;
            let items = list_ref(vm, argc)?;
            if idx >= 0 && (idx as usize) < items.len() {
                let v = items[idx as usize];
                make_some(vm, v)
            } else {
                make_none(vm)
            }
        }
        ListFirst => match list_ref(vm, argc)?.first().copied() {
            Some(v) => make_some(vm, v),
            None => make_none(vm),
        },
        ListLast => match list_ref(vm, argc)?.last().copied() {
            Some(v) => make_some(vm, v),
            None => make_none(vm),
        },
        ListContains => {
            // value_eq is &self, so the receiver borrow can live across it.
            let needle = vm.native_arg(argc, 1);
            let mut found = false;
            for &it in list_ref(vm, argc)? {
                if vm.value_eq(it, needle, 0).map_err(|m| vm.error(m))? {
                    found = true;
                    break;
                }
            }
            Value::Bool(found)
        }
        ListIndexOf => {
            let needle = vm.native_arg(argc, 1);
            let mut found = None;
            for (i, &it) in list_ref(vm, argc)?.iter().enumerate() {
                if vm.value_eq(it, needle, 0).map_err(|m| vm.error(m))? {
                    found = Some(i as i64);
                    break;
                }
            }
            match found {
                Some(i) => make_some(vm, Value::Int(i)),
                None => make_none(vm),
            }
        }
        ListReverse => {
            let mut items = list_ref(vm, argc)?.clone();
            items.reverse();
            make_list_prerooted(vm, items)
        }
        ListSort => {
            let mut items = list_ref(vm, argc)?.clone();
            sort_scalars(vm, &mut items)?;
            make_list_prerooted(vm, items)
        }
        ListSortBy => {
            let f = vm.native_arg(argc, 1);
            let items = list_ref(vm, argc)?.clone();
            let sorted = merge_sort_by(vm, items, f)?;
            make_list(vm, sorted)
        }
        ListMap => {
            let f = vm.native_arg(argc, 1);
            let items = snapshot_items(vm, argc)?;
            let out = alloc_rooted_list(vm, Vec::with_capacity(items.len()));
            for &item in &items {
                let r = vm.call_value(f, &[item])?;
                push_into(vm, out, r);
            }
            finish_rooted(vm, items.len() + 1, Value::Obj(out))
        }
        ListFilter => {
            let f = vm.native_arg(argc, 1);
            let items = snapshot_items(vm, argc)?;
            let out = alloc_rooted_list(vm, Vec::new());
            for &item in &items {
                let r = vm.call_value(f, &[item])?;
                let keep = expect_bool(vm, r)?;
                if keep {
                    push_into(vm, out, item);
                }
            }
            finish_rooted(vm, items.len() + 1, Value::Obj(out))
        }
        ListEach => {
            let f = vm.native_arg(argc, 1);
            let items = snapshot_items(vm, argc)?;
            for &item in &items {
                vm.call_value(f, &[item])?;
            }
            finish_rooted(vm, items.len(), Value::Unit)
        }
        ListFold => {
            let init = vm.native_arg(argc, 1);
            let f = vm.native_arg(argc, 2);
            let items = snapshot_items(vm, argc)?;
            vm.temp_roots.push(init);
            let mut acc = init;
            for &item in &items {
                acc = vm.call_value(f, &[acc, item])?;
                // Keep the fresh accumulator rooted across the next call.
                let top = vm.temp_roots.len() - 1;
                vm.temp_roots[top] = acc;
            }
            finish_rooted(vm, items.len() + 1, acc)
        }
        ListAny | ListAll => {
            let f = vm.native_arg(argc, 1);
            let items = snapshot_items(vm, argc)?;
            let mut result = matches!(n, ListAll);
            for &item in &items {
                let r = vm.call_value(f, &[item])?;
                let b = expect_bool(vm, r)?;
                if matches!(n, ListAny) && b {
                    result = true;
                    break;
                }
                if matches!(n, ListAll) && !b {
                    result = false;
                    break;
                }
            }
            finish_rooted(vm, items.len(), Value::Bool(result))
        }
        ListFind => {
            let f = vm.native_arg(argc, 1);
            let items = snapshot_items(vm, argc)?;
            let mut found = None;
            for &item in &items {
                let r = vm.call_value(f, &[item])?;
                if expect_bool(vm, r)? {
                    found = Some(item);
                    break;
                }
            }
            let r = match found {
                Some(v) => make_some(vm, v),
                None => make_none(vm),
            };
            finish_rooted(vm, items.len(), r)
        }
        ListFlatMap => {
            let f = vm.native_arg(argc, 1);
            let items = snapshot_items(vm, argc)?;
            let out = alloc_rooted_list(vm, Vec::new());
            for &item in &items {
                let r = vm.call_value(f, &[item])?;
                let Value::Obj(rh) = r else {
                    return Err(vm.error("internal: flat_map expects a List (VM bug)"));
                };
                let sub = match vm.heap.get(rh) {
                    Obj::List(items) => items.clone(),
                    _ => return Err(vm.error("internal: flat_map expects a List (VM bug)")),
                };
                for v in sub {
                    push_into(vm, out, v);
                }
            }
            finish_rooted(vm, items.len() + 1, Value::Obj(out))
        }
        ListZip => {
            // Both sources stay rooted on the stack and nothing user-visible
            // runs during the loop, so read them in place per index instead
            // of cloning both up front.
            let bh = expect_list(vm, vm.native_arg(argc, 1))?;
            let a_len = list_ref(vm, argc)?.len();
            let b_len = match vm.heap.get(bh) {
                Obj::List(items) => items.len(),
                _ => 0,
            };
            let n_pairs = a_len.min(b_len);
            let out = alloc_rooted_list(vm, Vec::with_capacity(n_pairs));
            for i in 0..n_pairs {
                let x = list_ref(vm, argc)?[i];
                let y = match vm.heap.get(bh) {
                    Obj::List(items) => items[i],
                    _ => Value::Unit,
                };
                let t = vm.alloc(Obj::Tuple(vec![x, y]));
                push_into(vm, out, Value::Obj(t));
            }
            finish_rooted(vm, 1, Value::Obj(out))
        }
        ListEnumerate => {
            let len = list_ref(vm, argc)?.len();
            let out = alloc_rooted_list(vm, Vec::with_capacity(len));
            for i in 0..len {
                // The receiver on the stack keeps every element alive.
                let v = list_ref(vm, argc)?[i];
                let t = vm.alloc(Obj::Tuple(vec![Value::Int(i as i64), v]));
                push_into(vm, out, Value::Obj(t));
            }
            finish_rooted(vm, 1, Value::Obj(out))
        }
        ListSlice => {
            let a = int_arg(vm, argc, 1)?;
            let b = int_arg(vm, argc, 2)?;
            let items = list_ref(vm, argc)?;
            let len = items.len() as i64;
            let start = a.clamp(0, len) as usize;
            let end = b.clamp(0, len) as usize;
            let out: Vec<Value> =
                if start >= end { Vec::new() } else { items[start..end].to_vec() };
            make_list_prerooted(vm, out)
        }
        ListConcat => {
            let items = {
                let a = list_ref(vm, argc)?;
                let bh = expect_list(vm, vm.native_arg(argc, 1))?;
                let b = match vm.heap.get(bh) {
                    Obj::List(items) => items,
                    _ => unreachable!(),
                };
                let mut out = Vec::with_capacity(a.len() + b.len());
                out.extend_from_slice(a);
                out.extend_from_slice(b);
                out
            };
            make_list_prerooted(vm, items)
        }
        ListJoin => {
            // Borrow the separator and every element straight from the heap
            // (no per-element String clones), sized exactly up front. Nothing
            // allocates on the Fable heap until the borrows end.
            let s = {
                let sep = str_ref(vm, argc, 1)?;
                let items = list_ref(vm, argc)?;
                let mut cap = sep.len() * items.len().saturating_sub(1);
                for &it in items {
                    if let Value::Obj(h) = it {
                        if let Obj::Str(t) = vm.heap.get(h) {
                            cap += t.len();
                        }
                    }
                }
                let mut s = String::with_capacity(cap);
                for (i, &it) in items.iter().enumerate() {
                    if i > 0 {
                        s.push_str(sep);
                    }
                    match it {
                        Value::Obj(h) => match vm.heap.get(h) {
                            Obj::Str(t) => s.push_str(t),
                            _ => return Err(vm.error("internal: expected String (VM bug)")),
                        },
                        _ => return Err(vm.error("internal: expected String (VM bug)")),
                    }
                }
                s
            };
            vm.alloc_str(s)
        }
        ListClone => {
            let items = list_ref(vm, argc)?.clone();
            make_list_prerooted(vm, items)
        }
        ListClear => {
            with_list_mut(vm, argc, |items| items.clear())?;
            Value::Unit
        }

        // ------------------------------------------------------------------
        // String methods
        // ------------------------------------------------------------------
        StrLen => Value::Int(str_ref(vm, argc, 0)?.chars().count() as i64),
        StrByteLen => Value::Int(str_ref(vm, argc, 0)?.len() as i64),
        StrIsEmpty => Value::Bool(str_ref(vm, argc, 0)?.is_empty()),
        StrChars => {
            let s = recv_str(vm, argc)?;
            let out = alloc_rooted_list(vm, Vec::new());
            for c in s.chars() {
                let cv = vm.char_str(c);
                push_into(vm, out, cv);
            }
            finish_rooted(vm, 1, Value::Obj(out))
        }
        StrSplit => {
            let s = recv_str(vm, argc)?;
            let sep = vm.str_of(vm.native_arg(argc, 1))?;
            let parts: Vec<String> = if sep.is_empty() {
                s.chars().map(|c| c.to_string()).collect()
            } else {
                s.split(&sep).map(|p| p.to_string()).collect()
            };
            let out = alloc_rooted_list(vm, Vec::new());
            for p in parts {
                let pv = vm.alloc_str(p);
                push_into(vm, out, pv);
            }
            finish_rooted(vm, 1, Value::Obj(out))
        }
        StrTrim => {
            let s = str_ref(vm, argc, 0)?.trim().to_string();
            vm.alloc_str(s)
        }
        StrTrimStart => {
            let s = str_ref(vm, argc, 0)?.trim_start().to_string();
            vm.alloc_str(s)
        }
        StrTrimEnd => {
            let s = str_ref(vm, argc, 0)?.trim_end().to_string();
            vm.alloc_str(s)
        }
        StrToUpper => {
            let s = str_ref(vm, argc, 0)?.to_ascii_uppercase();
            vm.alloc_str(s)
        }
        StrToLower => {
            let s = str_ref(vm, argc, 0)?.to_ascii_lowercase();
            vm.alloc_str(s)
        }
        StrContains => {
            let s = str_ref(vm, argc, 0)?;
            let sub = str_ref(vm, argc, 1)?;
            Value::Bool(s.contains(sub))
        }
        StrStartsWith => {
            let s = str_ref(vm, argc, 0)?;
            let sub = str_ref(vm, argc, 1)?;
            Value::Bool(s.starts_with(sub))
        }
        StrEndsWith => {
            let s = str_ref(vm, argc, 0)?;
            let sub = str_ref(vm, argc, 1)?;
            Value::Bool(s.ends_with(sub))
        }
        StrReplace => {
            let out = {
                let s = str_ref(vm, argc, 0)?;
                let from = str_ref(vm, argc, 1)?;
                let to = str_ref(vm, argc, 2)?;
                if from.is_empty() { s.to_string() } else { s.replace(from, to) }
            };
            vm.alloc_str(out)
        }
        StrSlice => {
            let a = int_arg(vm, argc, 1)?;
            let b = int_arg(vm, argc, 2)?;
            // Single forward walk over the borrowed receiver; clamping to the
            // char count falls out of skip/take saturating at the end.
            let out: String = {
                let s = str_ref(vm, argc, 0)?;
                let start = a.max(0) as usize;
                let count = (b.max(0) as usize).saturating_sub(start);
                s.chars().skip(start).take(count).collect()
            };
            vm.alloc_str(out)
        }
        StrCharAt => {
            let i = int_arg(vm, argc, 1)?;
            let c = if i < 0 {
                None
            } else {
                str_ref(vm, argc, 0)?.chars().nth(i as usize)
            };
            match c {
                Some(c) => {
                    let cv = vm.char_str(c);
                    make_some(vm, cv)
                }
                None => make_none(vm),
            }
        }
        StrCodeAt => {
            let i = int_arg(vm, argc, 1)?;
            let c = if i < 0 {
                None
            } else {
                str_ref(vm, argc, 0)?.chars().nth(i as usize)
            };
            match c {
                Some(c) => make_some(vm, Value::Int(c as i64)),
                None => make_none(vm),
            }
        }
        StrIndexOf => {
            let hit = {
                let s = str_ref(vm, argc, 0)?;
                let sub = str_ref(vm, argc, 1)?;
                s.find(sub).map(|byte_idx| s[..byte_idx].chars().count() as i64)
            };
            match hit {
                Some(char_idx) => make_some(vm, Value::Int(char_idx)),
                None => make_none(vm),
            }
        }
        StrIndexOfFrom => {
            let s = str_ref(vm, argc, 0)?;
            let sub = str_ref(vm, argc, 1)?;
            let from = int_arg(vm, argc, 2)?.max(0) as usize;
            // `from` is a character index, like every string index in Fable.
            // Past-the-end starts find nothing (except the empty pattern,
            // which matches at the very end).
            let byte_from = s
                .char_indices()
                .nth(from)
                .map(|(b, _)| b)
                .or(if from >= s.chars().count() { Some(s.len()) } else { None });
            let hit = byte_from.and_then(|bf| {
                if bf == s.len() && !sub.is_empty() {
                    None
                } else {
                    s[bf..].find(sub).map(|rel| {
                        s[..bf + rel].chars().count() as i64
                    })
                }
            });
            match hit {
                Some(char_idx) => make_some(vm, Value::Int(char_idx)),
                None => make_none(vm),
            }
        }
        StrRepeat => {
            let times = int_arg(vm, argc, 1)?;
            let out = {
                let s = str_ref(vm, argc, 0)?;
                if times <= 0 {
                    String::new()
                } else {
                    let total = (times as u128) * (s.len() as u128);
                    if total > (1 << 30) {
                        return Err(vm.error("string repeat result too large"));
                    }
                    s.repeat(times as usize)
                }
            };
            vm.alloc_str(out)
        }
        StrPadLeft | StrPadRight => {
            let width = int_arg(vm, argc, 1)?.max(0) as usize;
            let out = {
                let s = str_ref(vm, argc, 0)?;
                let pad = str_ref(vm, argc, 2)?;
                if width > (1 << 30) {
                    return Err(vm.error("pad width too large"));
                }
                let cur = s.chars().count();
                if cur >= width || pad.is_empty() {
                    s.to_string()
                } else {
                    let need = width - cur;
                    let filler: String = pad.chars().cycle().take(need).collect();
                    if matches!(n, StrPadLeft) {
                        format!("{filler}{s}")
                    } else {
                        format!("{s}{filler}")
                    }
                }
            };
            vm.alloc_str(out)
        }
        StrParseInt => {
            match str_ref(vm, argc, 0)?.parse::<i64>() {
                Ok(i) => make_some(vm, Value::Int(i)),
                Err(_) => make_none(vm),
            }
        }
        StrParseFloat => {
            match str_ref(vm, argc, 0)?.parse::<f64>() {
                Ok(f) => make_some(vm, Value::Float(f)),
                Err(_) => make_none(vm),
            }
        }
        StrParseHex => {
            let s = str_ref(vm, argc, 0)?;
            let digits = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
            // The raw 64-bit bit pattern, like the hex literal syntax and
            // `to_hex`'s output — no sign, so `n.to_hex().parse_hex() ==
            // Some(n)` round-trips for every Int, including negative ones.
            match u64::from_str_radix(digits, 16) {
                Ok(u) => make_some(vm, Value::Int(u as i64)),
                Err(_) => make_none(vm),
            }
        }
        StrToString => vm.native_arg(argc, 0),

        // ------------------------------------------------------------------
        // Map methods
        // ------------------------------------------------------------------
        MapLen => Value::Int(map_ref(vm, argc)?.len() as i64),
        MapIsEmpty => Value::Bool(map_ref(vm, argc)?.is_empty()),
        MapGet => {
            let key = vm.native_arg(argc, 1);
            let hash = vm.hash_value(key, 0).map_err(|m| vm.error(m))?;
            let found = {
                let m = map_ref(vm, argc)?;
                vm.map_find(m, hash, key)
                    .map_err(|m| vm.error(m))?
                    .map(|i| m.entries[i as usize].2)
            };
            match found {
                Some(v) => make_some(vm, v),
                None => make_none(vm),
            }
        }
        MapInsert => {
            let key = vm.native_arg(argc, 1);
            let v = vm.native_arg(argc, 2);
            let h = map_handle(vm, argc)?;
            match vm.map_insert(h, key, v)? {
                Some(prev) => make_some(vm, prev),
                None => make_none(vm),
            }
        }
        MapRemove => {
            let key = vm.native_arg(argc, 1);
            let hash = vm.hash_value(key, 0).map_err(|m| vm.error(m))?;
            let h = map_handle(vm, argc)?;
            let found = {
                let m = map_ref(vm, argc)?;
                vm.map_find(m, hash, key).map_err(|m| vm.error(m))?
            };
            match found {
                Some(i) => {
                    let removed = match vm.heap.get_mut(h) {
                        Obj::Map(m) => m.remove_at(i).2,
                        _ => Value::Unit,
                    };
                    make_some(vm, removed)
                }
                None => make_none(vm),
            }
        }
        MapContainsKey => {
            let key = vm.native_arg(argc, 1);
            let hash = vm.hash_value(key, 0).map_err(|m| vm.error(m))?;
            let m = map_ref(vm, argc)?;
            Value::Bool(vm.map_find(m, hash, key).map_err(|m| vm.error(m))?.is_some())
        }
        MapKeys | MapValues => {
            // No allocation happens while the borrow is live, so build the
            // result Vec straight from the entries and allocate once.
            let items: Vec<Value> = map_ref(vm, argc)?
                .entries
                .iter()
                .map(|&(_, k, v)| if matches!(n, MapKeys) { k } else { v })
                .collect();
            make_list_prerooted(vm, items)
        }
        MapEntries => {
            // The receiver on the stack keeps every key/value alive across
            // the per-entry tuple allocations.
            let len = map_ref(vm, argc)?.entries.len();
            let out = alloc_rooted_list(vm, Vec::with_capacity(len));
            for i in 0..len {
                let (_, k, v) = map_ref(vm, argc)?.entries[i];
                let t = vm.alloc(Obj::Tuple(vec![k, v]));
                push_into(vm, out, Value::Obj(t));
            }
            finish_rooted(vm, 1, Value::Obj(out))
        }
        MapClear => {
            let h = map_handle(vm, argc)?;
            if let Obj::Map(m) = vm.heap.get_mut(h) {
                m.clear();
            }
            Value::Unit
        }
        MapClone => {
            let m = map_ref(vm, argc)?.clone();
            Value::Obj(vm.alloc(Obj::Map(m)))
        }

        // ------------------------------------------------------------------
        // Int / Float methods
        // ------------------------------------------------------------------
        IntToFloat => Value::Float(int_arg(vm, argc, 0)? as f64),
        IntToString => {
            let i = int_arg(vm, argc, 0)?;
            vm.alloc_str(i.to_string())
        }
        IntAbs => Value::Int(checked_abs(vm, int_arg(vm, argc, 0)?)?),
        IntPow => {
            let base = int_arg(vm, argc, 0)?;
            let exp = int_arg(vm, argc, 1)?;
            if exp < 0 {
                return Err(vm.error("negative exponent in integer pow"));
            }
            // Bases with |base| <= 1 never overflow, for any exponent.
            match base {
                0 => Value::Int(if exp == 0 { 1 } else { 0 }),
                1 => Value::Int(1),
                -1 => Value::Int(if exp % 2 == 0 { 1 } else { -1 }),
                _ => {
                    if exp > u32::MAX as i64 {
                        return Err(vm.error("integer overflow"));
                    }
                    Value::Int(
                        base.checked_pow(exp as u32)
                            .ok_or_else(|| vm.error("integer overflow"))?,
                    )
                }
            }
        }
        IntMin => Value::Int(int_arg(vm, argc, 0)?.min(int_arg(vm, argc, 1)?)),
        IntMax => Value::Int(int_arg(vm, argc, 0)?.max(int_arg(vm, argc, 1)?)),

        // Bit intrinsics (v0.7 efficiency pass): straight onto the Rust
        // i64/u64 methods. Both zero-count methods return 64 for 0.
        IntCountOnes => Value::Int(int_arg(vm, argc, 0)?.count_ones() as i64),
        IntLeadingZeros => Value::Int(int_arg(vm, argc, 0)?.leading_zeros() as i64),
        IntTrailingZeros => Value::Int(int_arg(vm, argc, 0)?.trailing_zeros() as i64),
        IntUshr => {
            // Logical (zero-filling) right shift — `>>`'s panic contract.
            let x = int_arg(vm, argc, 0)?;
            let sh = int_arg(vm, argc, 1)?;
            if !(0..64).contains(&sh) {
                return Err(vm.error(format!(
                    "shift amount out of range: {sh} (must be 0..=63)"
                )));
            }
            Value::Int(((x as u64) >> sh) as i64)
        }
        IntRotateLeft | IntRotateRight => {
            let x = int_arg(vm, argc, 0)?;
            // Counts wrap mod 64 (Rust rotate semantics) — unlike shifts,
            // rotates never panic; a negative count rotates the other way.
            let sh = int_arg(vm, argc, 1)?.rem_euclid(64) as u32;
            Value::Int(if matches!(n, IntRotateLeft) {
                x.rotate_left(sh)
            } else {
                x.rotate_right(sh)
            })
        }
        IntToHex => {
            // Lowercase minimal hex of the two's-complement bit pattern:
            // (-1).to_hex() == "ffffffffffffffff", 0.to_hex() == "0".
            let x = int_arg(vm, argc, 0)? as u64;
            vm.alloc_str(format!("{x:x}"))
        }
        IntWrappingAdd => {
            Value::Int(int_arg(vm, argc, 0)?.wrapping_add(int_arg(vm, argc, 1)?))
        }
        IntWrappingSub => {
            Value::Int(int_arg(vm, argc, 0)?.wrapping_sub(int_arg(vm, argc, 1)?))
        }
        IntWrappingMul => {
            Value::Int(int_arg(vm, argc, 0)?.wrapping_mul(int_arg(vm, argc, 1)?))
        }

        FloatToInt => {
            let f = float_arg(vm, argc, 0)?;
            if f.is_nan() {
                return Err(vm.error("cannot convert nan to Int"));
            }
            let t = f.trunc();
            if !(-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&t) {
                return Err(vm.error(format!("float {} is out of Int range", fmt_float(f))));
            }
            Value::Int(t as i64)
        }
        FloatToString => {
            let f = float_arg(vm, argc, 0)?;
            vm.alloc_str(fmt_float(f))
        }
        FloatAbs => Value::Float(float_arg(vm, argc, 0)?.abs()),
        FloatFloor => Value::Float(float_arg(vm, argc, 0)?.floor()),
        FloatCeil => Value::Float(float_arg(vm, argc, 0)?.ceil()),
        FloatRound => Value::Float(float_arg(vm, argc, 0)?.round()),
        FloatSqrt => Value::Float(float_arg(vm, argc, 0)?.sqrt()),
        FloatIsNan => Value::Bool(float_arg(vm, argc, 0)?.is_nan()),
        FloatToFixed => {
            let f = float_arg(vm, argc, 0)?;
            let places = int_arg(vm, argc, 1)?.clamp(0, 17) as usize;
            let mut out = format!("{f:.places$}");
            // A value that rounds to zero displays as zero, not "-0.00" —
            // labels built from rounded floats shouldn't grow stray signs.
            if out.starts_with('-') && out[1..].chars().all(|c| c == '0' || c == '.') {
                out.remove(0);
            }
            vm.alloc_str(out)
        }

        // ------------------------------------------------------------------
        // Option methods
        // ------------------------------------------------------------------
        OptIsSome => Value::Bool(variant_of(vm, argc)? == OPTION_SOME),
        OptIsNone => Value::Bool(variant_of(vm, argc)? == OPTION_NONE),
        OptUnwrap => {
            if variant_of(vm, argc)? == OPTION_NONE {
                return Err(vm.error("called `unwrap()` on `None`"));
            }
            variant_field(vm, argc, 0)?
        }
        OptUnwrapOr => {
            if variant_of(vm, argc)? == OPTION_SOME {
                variant_field(vm, argc, 0)?
            } else {
                vm.native_arg(argc, 1)
            }
        }
        OptMap => {
            let f = vm.native_arg(argc, 1);
            if variant_of(vm, argc)? == OPTION_SOME {
                let v = variant_field(vm, argc, 0)?;
                let r = vm.call_value(f, &[v])?;
                make_some(vm, r)
            } else {
                make_none(vm)
            }
        }
        OptAndThen => {
            let f = vm.native_arg(argc, 1);
            if variant_of(vm, argc)? == OPTION_SOME {
                let v = variant_field(vm, argc, 0)?;
                vm.call_value(f, &[v])?
            } else {
                make_none(vm)
            }
        }
        OptOr => {
            if variant_of(vm, argc)? == OPTION_SOME {
                vm.native_arg(argc, 0)
            } else {
                vm.native_arg(argc, 1)
            }
        }

        // ------------------------------------------------------------------
        // Result methods
        // ------------------------------------------------------------------
        ResIsOk => Value::Bool(variant_of(vm, argc)? == RESULT_OK),
        ResIsErr => Value::Bool(variant_of(vm, argc)? == RESULT_ERR),
        ResUnwrap => {
            if variant_of(vm, argc)? == RESULT_ERR {
                let e = variant_field(vm, argc, 0)?;
                let es = vm.display_value(e)?;
                return Err(vm.error(format!("called `unwrap()` on an `Err`: {es}")));
            }
            variant_field(vm, argc, 0)?
        }
        ResUnwrapOr => {
            if variant_of(vm, argc)? == RESULT_OK {
                variant_field(vm, argc, 0)?
            } else {
                vm.native_arg(argc, 1)
            }
        }
        ResUnwrapErr => {
            if variant_of(vm, argc)? == RESULT_OK {
                let v = variant_field(vm, argc, 0)?;
                let vs = vm.display_value(v)?;
                return Err(vm.error(format!("called `unwrap_err()` on an `Ok`: {vs}")));
            }
            variant_field(vm, argc, 0)?
        }
        ResMap => {
            let f = vm.native_arg(argc, 1);
            if variant_of(vm, argc)? == RESULT_OK {
                let v = variant_field(vm, argc, 0)?;
                let r = vm.call_value(f, &[v])?;
                make_ok(vm, r)
            } else {
                vm.native_arg(argc, 0)
            }
        }
        ResMapErr => {
            let f = vm.native_arg(argc, 1);
            if variant_of(vm, argc)? == RESULT_ERR {
                let v = variant_field(vm, argc, 0)?;
                let r = vm.call_value(f, &[v])?;
                make_err(vm, r)
            } else {
                vm.native_arg(argc, 0)
            }
        }
        ResAndThen => {
            let f = vm.native_arg(argc, 1);
            if variant_of(vm, argc)? == RESULT_OK {
                let v = variant_field(vm, argc, 0)?;
                vm.call_value(f, &[v])?
            } else {
                vm.native_arg(argc, 0)
            }
        }

        // ------------------------------------------------------------------
        // Range methods
        // ------------------------------------------------------------------
        RangeToList | RangeRev => {
            let (lo, hi, inclusive) = range_of(vm, argc)?;
            let count = range_count(lo, hi, inclusive);
            if count > 100_000_000 {
                return Err(vm.error("range too large to materialize"));
            }
            let mut items: Vec<Value> = Vec::with_capacity(count as usize);
            for k in 0..count as i64 {
                // lo + k <= hi, so this never overflows.
                items.push(Value::Int(lo + k));
            }
            if matches!(n, RangeRev) {
                items.reverse();
            }
            make_list_prerooted(vm, items)  // all Ints: nothing to root
        }
        RangeContains => {
            let (lo, hi, inclusive) = range_of(vm, argc)?;
            let v = int_arg(vm, argc, 1)?;
            Value::Bool(v >= lo && if inclusive { v <= hi } else { v < hi })
        }
        RangeLen => {
            let (lo, hi, inclusive) = range_of(vm, argc)?;
            let count = range_count(lo, hi, inclusive);
            if count > i64::MAX as i128 {
                return Err(vm.error("integer overflow (range length does not fit in Int)"));
            }
            Value::Int(count as i64)
        }
        RangeMap | RangeFilter | RangeEach => {
            let f = vm.native_arg(argc, 1);
            let (lo, hi, inclusive) = range_of(vm, argc)?;
            let count = range_count(lo, hi, inclusive);
            let out = alloc_rooted_list(vm, Vec::new());
            for k in 0..count {
                let item = Value::Int(lo.wrapping_add(k as i64));
                match n {
                    RangeMap => {
                        let r = vm.call_value(f, &[item])?;
                        push_into(vm, out, r);
                    }
                    RangeFilter => {
                        let r = vm.call_value(f, &[item])?;
                if expect_bool(vm, r)? {
                            push_into(vm, out, item);
                        }
                    }
                    _ => {
                        vm.call_value(f, &[item])?;
                    }
                }
            }
            let result = if matches!(n, RangeEach) { Value::Unit } else { Value::Obj(out) };
            finish_rooted(vm, 1, result)
        }
        RangeFold => {
            let init = vm.native_arg(argc, 1);
            let f = vm.native_arg(argc, 2);
            let (lo, hi, inclusive) = range_of(vm, argc)?;
            let count = range_count(lo, hi, inclusive);
            vm.temp_roots.push(init);
            let mut acc = init;
            for k in 0..count {
                acc = vm.call_value(f, &[acc, Value::Int(lo.wrapping_add(k as i64))])?;
                let top = vm.temp_roots.len() - 1;
                vm.temp_roots[top] = acc;
            }
            finish_rooted(vm, 1, acc)
        }
        RangeAny | RangeAll => {
            // Short-circuiting, unlike map/filter/each — every Range item is
            // a plain Int (never a heap object), so no GC rooting is needed.
            let f = vm.native_arg(argc, 1);
            let (lo, hi, inclusive) = range_of(vm, argc)?;
            let count = range_count(lo, hi, inclusive);
            let mut result = matches!(n, RangeAll);
            for k in 0..count {
                let item = Value::Int(lo.wrapping_add(k as i64));
                let r = vm.call_value(f, &[item])?;
                let b = expect_bool(vm, r)?;
                if matches!(n, RangeAny) && b {
                    result = true;
                    break;
                }
                if matches!(n, RangeAll) && !b {
                    result = false;
                    break;
                }
            }
            Value::Bool(result)
        }
    };
    vm.finish_native(argc, result);
    Ok(())
}

// ---------------------------------------------------------------------------
// Argument helpers
// ---------------------------------------------------------------------------

fn str_arg(vm: &Vm, argc: u8, i: u8) -> Result<String, VmError> {
    vm.str_of(vm.native_arg(argc, i))
}

/// Borrow the String argument at position `i` straight from the heap — the
/// no-clone counterpart of `str_arg` for natives that only need to read it.
fn str_ref(vm: &Vm, argc: u8, i: u8) -> Result<&str, VmError> {
    match vm.native_arg(argc, i) {
        Value::Obj(h) => match vm.heap.get(h) {
            Obj::Str(s) => Ok(s),
            _ => Err(vm.error("internal: expected String (VM bug)")),
        },
        _ => Err(vm.error("internal: expected String (VM bug)")),
    }
}

fn list_arg(vm: &Vm, argc: u8, i: u8) -> Result<Vec<Value>, VmError> {
    match vm.native_arg(argc, i) {
        Value::Obj(h) => match vm.heap.get(h) {
            Obj::List(items) => Ok(items.clone()),
            _ => Err(vm.error("internal: expected List argument (VM bug)")),
        },
        _ => Err(vm.error("internal: expected List argument (VM bug)")),
    }
}

/// An `Err` carrying "<path>: <os error>".
fn make_io_err(vm: &mut Vm, path: &str, e: std::io::Error) -> Value {
    let msg = vm.alloc_str(format!("{path}: {e}"));
    make_err(vm, msg)
}

fn int_arg(vm: &Vm, argc: u8, i: u8) -> Result<i64, VmError> {
    match vm.native_arg(argc, i) {
        Value::Int(v) => Ok(v),
        _ => Err(vm.error("internal: expected Int argument (VM bug)")),
    }
}

fn float_arg(vm: &Vm, argc: u8, i: u8) -> Result<f64, VmError> {
    match vm.native_arg(argc, i) {
        Value::Float(v) => Ok(v),
        _ => Err(vm.error("internal: expected Float argument (VM bug)")),
    }
}

fn expect_bool(vm: &Vm, v: Value) -> Result<bool, VmError> {
    match v {
        Value::Bool(b) => Ok(b),
        _ => Err(vm.error("internal: expected Bool (VM bug)")),
    }
}

fn expect_list(vm: &Vm, v: Value) -> Result<Handle, VmError> {
    match v {
        Value::Obj(h) if matches!(vm.heap.get(h), Obj::List(_)) => Ok(h),
        _ => Err(vm.error("internal: expected List (VM bug)")),
    }
}



fn float_vec_arg(vm: &Vm, argc: u8, i: u8) -> Result<Vec<f64>, VmError> {
    match vm.native_arg(argc, i) {
        Value::Obj(h) => match vm.heap.get(h) {
            Obj::List(items) => {
                let mut out = Vec::with_capacity(items.len());
                for v in items {
                    match v {
                        Value::Float(f) => out.push(*f),
                        _ => return Err(vm.error("expected a List[Float]")),
                    }
                }
                Ok(out)
            }
            _ => Err(vm.error("expected a List[Float]")),
        },
        _ => Err(vm.error("expected a List[Float]")),
    }
}

/// Allocate `(List[Float], List[Float])` with GC-safe rooting.
fn floats_pair(vm: &mut Vm, a: Vec<f64>, b: Vec<f64>) -> Value {
    let la = alloc_rooted_list(vm, a.into_iter().map(Value::Float).collect());
    let lb = alloc_rooted_list(vm, b.into_iter().map(Value::Float).collect());
    let t = vm.heap.alloc(Obj::Tuple(vec![Value::Obj(la), Value::Obj(lb)]));
    finish_rooted(vm, 2, Value::Obj(t))
}

fn bytes_handle(vm: &Vm, argc: u8) -> Result<Handle, VmError> {
    match vm.native_arg(argc, 0) {
        Value::Obj(h) => match vm.heap.get(h) {
            Obj::Bytes(_) => Ok(h),
            _ => Err(vm.error("expected Bytes")),
        },
        _ => Err(vm.error("expected Bytes")),
    }
}

fn bytes_ref(vm: &Vm, argc: u8) -> Result<&Vec<u8>, VmError> {
    match vm.native_arg(argc, 0) {
        Value::Obj(h) => match vm.heap.get(h) {
            Obj::Bytes(bs) => Ok(bs),
            _ => Err(vm.error("expected Bytes")),
        },
        _ => Err(vm.error("expected Bytes")),
    }
}

/// The receiver's worker handle, cloned out of the heap (`Rc`) so callers
/// can block on it without holding a heap borrow.
fn worker_rc(
    vm: &Vm,
    argc: u8,
) -> Result<std::rc::Rc<std::cell::RefCell<crate::worker::WorkerHandle>>, VmError> {
    match vm.native_arg(argc, 0) {
        Value::Obj(h) => match vm.heap.get(h) {
            Obj::Worker(rc) => Ok(rc.clone()),
            _ => Err(vm.error("expected Worker")),
        },
        _ => Err(vm.error("expected Worker")),
    }
}

/// The receiver's window handle, cloned out of the heap (`Rc`), mirroring
/// `worker_rc` exactly.
fn window_rc(
    vm: &Vm,
    argc: u8,
) -> Result<std::rc::Rc<std::cell::RefCell<crate::window::WindowHandle>>, VmError> {
    match vm.native_arg(argc, 0) {
        Value::Obj(h) => match vm.heap.get(h) {
            Obj::Window(rc) => Ok(rc.clone()),
            _ => Err(vm.error("expected Window")),
        },
        _ => Err(vm.error("expected Window")),
    }
}

fn list_handle(vm: &Vm, argc: u8) -> Result<Handle, VmError> {
    expect_list(vm, vm.native_arg(argc, 0))
}

// ---------------------------------------------------------------------------
// gfx.* (v0.8) argument helpers
// ---------------------------------------------------------------------------

/// Resolve the currently-current GL window as a plain `String` error —
/// shared by every `gfx.*` native. `compile_program` folds this straight
/// into its own `Result` (it's the one member with a `Result` return);
/// `gfx_window` below wraps it as a `VmError` (panic) for everything else,
/// matching `window`'s own methods' "assumes valid state" shape.
///
/// Distinguishes two causes with two distinct messages: the `gl` cargo
/// feature being off entirely (`cfg!` is a compile-time constant, so this
/// check costs nothing and is accurate for the running binary) vs. no
/// window ever having called `win.make_current()` yet.
fn gfx_window_msg(
    vm: &Vm,
) -> Result<std::rc::Rc<std::cell::RefCell<crate::window::WindowHandle>>, String> {
    // Checking `gl` alone would wrongly report every `gfx.*` call as "not
    // compiled in" on a `--features metal`- or `--features vulkan`-only
    // build, even with a live Metal/Vulkan window current -- `gfx` is
    // backend-neutral, so it's compiled in whenever any rendering backend
    // is. (Vulkan windowing is Linux-only, hence the platform-qualified
    // check: on a Windows `--features vulkan` build only the compute path
    // exists, and `gfx.*` really isn't compiled in.)
    if !cfg!(feature = "gl")
        && !cfg!(feature = "metal")
        && !cfg!(all(feature = "vulkan", target_os = "linux"))
    {
        return Err(
            "gfx support not compiled in (build with --features gl, metal, or vulkan)"
                .to_string(),
        );
    }
    match &vm.gfx_current_window {
        Some(w) => Ok(w.clone()),
        None => Err("gfx: no current GL context -- call window.make_current() first".to_string()),
    }
}

/// `gfx_window_msg`, panicking (`vm.error`) instead of returning a `String`
/// — every `gfx.*` native except `compile_program` uses this, since they
/// have no `Result` to carry an `Err` through.
fn gfx_window(
    vm: &Vm,
) -> Result<std::rc::Rc<std::cell::RefCell<crate::window::WindowHandle>>, VmError> {
    gfx_window_msg(vm).map_err(|m| vm.error(m))
}

/// A GL object handle (`Int` on the Fable side) as `u32` — every
/// `gfx.*` shader/program/buffer/VAO/texture parameter.
fn u32_arg(vm: &Vm, argc: u8, i: u8) -> Result<u32, VmError> {
    let v = int_arg(vm, argc, i)?;
    u32::try_from(v).map_err(|_| vm.error(format!("gfx: Int {v} is out of range for a GL handle")))
}

/// A GL size/offset/count (`Int` on the Fable side) as `i32`.
fn i32_arg(vm: &Vm, argc: u8, i: u8) -> Result<i32, VmError> {
    let v = int_arg(vm, argc, i)?;
    i32::try_from(v).map_err(|_| vm.error(format!("gfx: Int {v} is out of i32 range")))
}

/// `gfx.bind_buffer`/`gfx.upload_buffer`'s `kind: String` ("vertex" or
/// "index") as the platform-neutral `GfxBufferKind` tag.
fn buffer_kind_arg(vm: &Vm, argc: u8, i: u8) -> Result<crate::window::GfxBufferKind, VmError> {
    let s = str_arg(vm, argc, i)?;
    match s.as_str() {
        "vertex" => Ok(crate::window::GfxBufferKind::Vertex),
        "index" => Ok(crate::window::GfxBufferKind::Index),
        other => Err(vm.error(format!(
            "gfx: buffer kind must be \"vertex\" or \"index\", got {other:?}"
        ))),
    }
}

/// A struct value's fields, cloned out of the heap — `None` if `v` isn't a
/// struct at all (used by `mat4_arg` to duck-type `Mat4`'s shape without
/// referencing its `DefId`, which natives can't name; see
/// `Native::GfxSetUniformMat4`'s doc comment).
fn struct_fields(vm: &Vm, v: Value) -> Option<Vec<Value>> {
    match v {
        Value::Obj(h) => match vm.heap.get(h) {
            Obj::Struct { fields, .. } => Some(fields.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// Flatten a `Mat4` argument (`std/glm.fable`: 4 `Vec4` columns `c0..c3`,
/// each an `{x, y, z, w}` struct of `Float`s — already column-major,
/// matching GL's own convention) into 16 column-major `f32`s for
/// `glUniformMatrix4fv`. `gfx.set_uniform_mat4`'s third parameter is a
/// fresh scheme variable, not a concrete `Mat4` (natives can't reference a
/// `std` module struct's `DefId`), so this duck-types the runtime shape
/// instead of relying on the static type — anything else is a clear panic,
/// not a silently wrong upload.
fn mat4_arg(vm: &Vm, argc: u8, i: u8) -> Result<[f32; 16], VmError> {
    let bad = || vm.error("gfx.set_uniform_mat4: expected a Mat4 (4 Vec4 columns of 4 Floats)");
    let cols = struct_fields(vm, vm.native_arg(argc, i)).ok_or_else(bad)?;
    if cols.len() != 4 {
        return Err(bad());
    }
    let mut out = [0f32; 16];
    for (ci, col) in cols.into_iter().enumerate() {
        let comps = struct_fields(vm, col).ok_or_else(bad)?;
        if comps.len() != 4 {
            return Err(bad());
        }
        for (fi, comp) in comps.into_iter().enumerate() {
            match comp {
                Value::Float(f) => out[ci * 4 + fi] = f as f32,
                _ => return Err(bad()),
            }
        }
    }
    Ok(out)
}

/// The Bytes argument at position `i` — for natives whose Bytes parameter is
/// not the receiver (e.g. `gpu.run`'s input). `bytes_ref` reads arg 0.
fn bytes_arg(vm: &Vm, argc: u8, i: u8) -> Result<&Vec<u8>, VmError> {
    match vm.native_arg(argc, i) {
        Value::Obj(h) => match vm.heap.get(h) {
            Obj::Bytes(bs) => Ok(bs),
            _ => Err(vm.error("expected Bytes")),
        },
        _ => Err(vm.error("expected Bytes")),
    }
}

fn list_ref(vm: &Vm, argc: u8) -> Result<&Vec<Value>, VmError> {
    match vm.native_arg(argc, 0) {
        Value::Obj(h) => match vm.heap.get(h) {
            Obj::List(items) => Ok(items),
            _ => Err(vm.error("internal: expected List (VM bug)")),
        },
        _ => Err(vm.error("internal: expected List (VM bug)")),
    }
}

/// Run `f` on a mutable borrow of the receiver list — one heap lookup
/// instead of `list_handle` + `heap.get_mut`, for in-place mutators.
fn with_list_mut<T>(
    vm: &mut Vm,
    argc: u8,
    f: impl FnOnce(&mut Vec<Value>) -> T,
) -> Result<T, VmError> {
    let r = match vm.native_arg(argc, 0) {
        Value::Obj(h) => match vm.heap.get_mut(h) {
            Obj::List(items) => Some(f(items)),
            _ => None,
        },
        _ => None,
    };
    r.ok_or_else(|| vm.error("internal: expected List (VM bug)"))
}

fn map_handle(vm: &Vm, argc: u8) -> Result<Handle, VmError> {
    match vm.native_arg(argc, 0) {
        Value::Obj(h) if matches!(vm.heap.get(h), Obj::Map(_)) => Ok(h),
        _ => Err(vm.error("internal: expected Map (VM bug)")),
    }
}

fn map_ref(vm: &Vm, argc: u8) -> Result<&FMap, VmError> {
    match vm.native_arg(argc, 0) {
        Value::Obj(h) => match vm.heap.get(h) {
            Obj::Map(m) => Ok(m),
            _ => Err(vm.error("internal: expected Map (VM bug)")),
        },
        _ => Err(vm.error("internal: expected Map (VM bug)")),
    }
}

fn recv_str(vm: &Vm, argc: u8) -> Result<String, VmError> {
    vm.str_of(vm.native_arg(argc, 0))
}

fn variant_of(vm: &Vm, argc: u8) -> Result<u32, VmError> {
    match vm.native_arg(argc, 0) {
        Value::Obj(h) => match vm.heap.get(h) {
            Obj::Variant { variant, .. } => Ok(*variant),
            _ => Err(vm.error("internal: expected enum value (VM bug)")),
        },
        _ => Err(vm.error("internal: expected enum value (VM bug)")),
    }
}

fn variant_field(vm: &Vm, argc: u8, i: usize) -> Result<Value, VmError> {
    match vm.native_arg(argc, 0) {
        Value::Obj(h) => match vm.heap.get(h) {
            Obj::Variant { fields, .. } => fields
                .get(i)
                .copied()
                .ok_or_else(|| vm.error("internal: missing variant field (VM bug)")),
            _ => Err(vm.error("internal: expected enum value (VM bug)")),
        },
        _ => Err(vm.error("internal: expected enum value (VM bug)")),
    }
}

fn range_of(vm: &Vm, argc: u8) -> Result<(i64, i64, bool), VmError> {
    match vm.native_arg(argc, 0) {
        Value::Obj(h) => match vm.heap.get(h) {
            Obj::Range { lo, hi, inclusive } => Ok((*lo, *hi, *inclusive)),
            _ => Err(vm.error("internal: expected Range (VM bug)")),
        },
        _ => Err(vm.error("internal: expected Range (VM bug)")),
    }
}

/// Number of elements in a range, exact (may exceed i64::MAX for inclusive
/// full-width ranges, hence i128).
fn range_count(lo: i64, hi: i64, inclusive: bool) -> i128 {
    let end = if inclusive { hi as i128 + 1 } else { hi as i128 };
    (end - lo as i128).max(0)
}

// ---------------------------------------------------------------------------
// Allocation helpers (rooting discipline)
// ---------------------------------------------------------------------------

fn make_some(vm: &mut Vm, v: Value) -> Value {
    vm.temp_roots.push(v);
    let h = vm.alloc(Obj::Variant { def: OPTION_DEF, variant: OPTION_SOME, fields: vec![v] });
    vm.temp_roots.pop();
    Value::Obj(h)
}

fn make_none(vm: &mut Vm) -> Value {
    let h = vm.alloc(Obj::Variant { def: OPTION_DEF, variant: OPTION_NONE, fields: vec![] });
    Value::Obj(h)
}

fn make_ok(vm: &mut Vm, v: Value) -> Value {
    vm.temp_roots.push(v);
    let h = vm.alloc(Obj::Variant { def: RESULT_DEF, variant: RESULT_OK, fields: vec![v] });
    vm.temp_roots.pop();
    Value::Obj(h)
}

fn make_err(vm: &mut Vm, v: Value) -> Value {
    vm.temp_roots.push(v);
    let h = vm.alloc(Obj::Variant { def: RESULT_DEF, variant: RESULT_ERR, fields: vec![v] });
    vm.temp_roots.pop();
    Value::Obj(h)
}

fn make_list(vm: &mut Vm, items: Vec<Value>) -> Value {
    let start = vm.temp_roots.len();
    vm.temp_roots.extend_from_slice(&items);
    let h = vm.alloc(Obj::List(items));
    vm.temp_roots.truncate(start);
    Value::Obj(h)
}

/// Allocate a list whose elements are all still reachable from the stack
/// (they were read from receiver/argument objects and no user code has run
/// since) — the GC checkpoint inside `alloc` can't free them, so the
/// temp-roots copy `make_list` does is skipped.
fn make_list_prerooted(vm: &mut Vm, items: Vec<Value>) -> Value {
    Value::Obj(vm.alloc(Obj::List(items)))
}

fn make_tuple(vm: &mut Vm, items: Vec<Value>) -> Value {
    let start = vm.temp_roots.len();
    vm.temp_roots.extend_from_slice(&items);
    let h = vm.alloc(Obj::Tuple(items));
    vm.temp_roots.truncate(start);
    Value::Obj(h)
}

/// Snapshot the receiver list into a Rust-local Vec whose elements are
/// pushed onto `temp_roots`, so the iteration source survives callbacks
/// that mutate or drop the original. The caller must pop `items.len()`
/// roots when done (its `finish_rooted` count includes them).
fn snapshot_items(vm: &mut Vm, argc: u8) -> Result<Vec<Value>, VmError> {
    let items = list_ref(vm, argc)?.clone();
    vm.temp_roots.extend_from_slice(&items);
    Ok(items)
}

/// Allocate a result list and temp-root it for the duration of a native.
fn alloc_rooted_list(vm: &mut Vm, items: Vec<Value>) -> Handle {
    let v = make_list(vm, items);
    vm.temp_roots.push(v);
    let Value::Obj(h) = v else { unreachable!() };
    h
}

fn push_into(vm: &mut Vm, list: Handle, v: Value) {
    if let Obj::List(items) = vm.heap.get_mut(list) {
        items.push(v);
    }
}

/// Pop `n` temp roots and return the result.
fn finish_rooted(vm: &mut Vm, n: usize, result: Value) -> Value {
    let len = vm.temp_roots.len() - n;
    vm.temp_roots.truncate(len);
    result
}

// ---------------------------------------------------------------------------
// Sorting
// ---------------------------------------------------------------------------

fn sort_scalars(vm: &Vm, items: &mut [Value]) -> Result<(), VmError> {
    if items.is_empty() {
        return Ok(());
    }
    match items[0] {
        Value::Int(_) => {
            for v in items.iter() {
                if !matches!(v, Value::Int(_)) {
                    return Err(vm.error("sort: mixed element types"));
                }
            }
            items.sort_by_key(|v| match v {
                Value::Int(i) => *i,
                _ => 0,
            });
            Ok(())
        }
        Value::Float(_) => {
            for v in items.iter() {
                if !matches!(v, Value::Float(_)) {
                    return Err(vm.error("sort: mixed element types"));
                }
            }
            items.sort_by(|a, b| match (a, b) {
                (Value::Float(x), Value::Float(y)) => x.total_cmp(y),
                _ => Ordering::Equal,
            });
            Ok(())
        }
        Value::Obj(h) if matches!(vm.heap.get(h), Obj::Str(_)) => {
            // Validate up front (same error str_of used to raise), then
            // compare borrowed strings in place — no per-element clones.
            for v in items.iter() {
                match v {
                    Value::Obj(h) if matches!(vm.heap.get(*h), Obj::Str(_)) => {}
                    _ => return Err(vm.error("internal: expected String (VM bug)")),
                }
            }
            items.sort_by(|a, b| match (a, b) {
                (Value::Obj(x), Value::Obj(y)) => {
                    match (vm.heap.get(*x), vm.heap.get(*y)) {
                        (Obj::Str(sx), Obj::Str(sy)) => sx.cmp(sy),
                        _ => Ordering::Equal,
                    }
                }
                _ => Ordering::Equal,
            });
            Ok(())
        }
        _ => Err(vm.error(
            "sort() requires Int, Float, or String elements; use sort_by(..) for other types",
        )),
    }
}

/// Stable merge sort with a user comparator (`fn(T, T) -> Int`).
fn merge_sort_by(vm: &mut Vm, items: Vec<Value>, f: Value) -> Result<Vec<Value>, VmError> {
    // Root everything for the duration (comparator calls can collect).
    let start = vm.temp_roots.len();
    vm.temp_roots.extend_from_slice(&items);
    let result = merge_sort_inner(vm, items, f);
    vm.temp_roots.truncate(start);
    result
}

fn merge_sort_inner(vm: &mut Vm, items: Vec<Value>, f: Value) -> Result<Vec<Value>, VmError> {
    if items.len() <= 1 {
        return Ok(items);
    }
    let mid = items.len() / 2;
    let mut left = items;
    let right = left.split_off(mid);
    let left = merge_sort_inner(vm, left, f)?;
    let right = merge_sort_inner(vm, right, f)?;
    let mut out = Vec::with_capacity(left.len() + right.len());
    let (mut i, mut j) = (0, 0);
    while i < left.len() && j < right.len() {
        let r = vm.call_value(f, &[left[i], right[j]])?;
        let Value::Int(ord) = r else {
            return Err(vm.error("internal: comparator must return Int (VM bug)"));
        };
        if ord <= 0 {
            out.push(left[i]);
            i += 1;
        } else {
            out.push(right[j]);
            j += 1;
        }
    }
    out.extend_from_slice(&left[i..]);
    out.extend_from_slice(&right[j..]);
    Ok(out)
}

fn checked_abs(vm: &Vm, i: i64) -> Result<i64, VmError> {
    i.checked_abs().ok_or_else(|| vm.error("integer overflow"))
}
