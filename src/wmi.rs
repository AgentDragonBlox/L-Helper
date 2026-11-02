//! Minimal WMI client over COM (root\WMI), enough for Lenovo's GameZone / OTHER_METHOD / FAN_METHOD classes.
//! COM objects aren't Send, so every thread gets its own connection (thread_local) and its own COM init.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use windows::core::{BSTR, PCWSTR};
use windows::Win32::System::Com::*;
use windows::Win32::System::Ole::*;
use windows::Win32::System::Variant::*;
use windows::Win32::System::Wmi::*;

pub type Result<T> = std::result::Result<T, String>;

const NAMESPACE: &str = "root\\WMI";

struct Conn {
    services: IWbemServices,
    paths: HashMap<String, Option<String>>,
}

thread_local! {
    static CONN: RefCell<Option<Conn>> = const { RefCell::new(None) };
}

/// Process-wide COM security; call once from main before any WMI use
pub fn init_security() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let _ = CoInitializeSecurity(None, -1, None, None, RPC_C_AUTHN_LEVEL_DEFAULT, RPC_C_IMP_LEVEL_IMPERSONATE, None, EOAC_NONE, None);
    }
}

fn connect() -> Result<IWbemServices> {
    connect_to(NAMESPACE)
}

fn connect_to(namespace: &str) -> Result<IWbemServices> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
        let services = locator
            .ConnectServer(&BSTR::from(namespace), &BSTR::new(), &BSTR::new(), &BSTR::new(), 0, &BSTR::new(), None)
            .map_err(|e| format!("ConnectServer: {e}"))?;
        // RPC_C_AUTHN_WINNT = 10, RPC_C_AUTHZ_NONE = 0
        let _ = CoSetProxyBlanket(&services, 10, 0, PCWSTR::null(), RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE, None, EOAC_NONE);
        Ok(services)
    }
}

fn with_conn<T>(f: impl FnOnce(&mut Conn) -> Result<T>) -> Result<T> {
    CONN.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = Some(Conn { services: connect()?, paths: HashMap::new() });
        }
        f(slot.as_mut().unwrap())
    })
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

// ---------------- values ----------------

/// A WMI property value converted out of a VARIANT
#[derive(Debug, Clone)]
pub enum Val {
    Null,
    Int(i64),
    Bool(bool),
    Str(String),
    Array(Vec<i64>),
}

impl Val {
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Val::Int(i) => Some(*i),
            Val::Bool(b) => Some(*b as i64),
            Val::Str(s) => s.trim().parse().ok(),
            _ => None,
        }
    }
    pub fn as_i32(&self) -> Option<i32> {
        self.as_i64().map(|v| v as i32)
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Val::Bool(b) => Some(*b),
            other => other.as_i64().map(|v| v != 0),
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Val::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_array(&self) -> Vec<i64> {
        match self {
            Val::Array(a) => a.clone(),
            _ => Vec::new(),
        }
    }
}

unsafe fn scalar(vt: VARENUM, data: &VARIANT_0_0_0) -> Option<i64> {
    Some(match vt {
        VT_I1 => data.cVal as i8 as i64,
        VT_UI1 => data.bVal as i64,
        VT_I2 => data.iVal as i64,
        VT_UI2 => data.uiVal as i64,
        VT_I4 | VT_INT => data.lVal as i64,
        VT_UI4 | VT_UINT => data.ulVal as i64,
        VT_I8 => data.llVal,
        VT_UI8 => data.ullVal as i64,
        VT_BOOL => (data.boolVal.0 != 0) as i64,
        _ => return None,
    })
}

unsafe fn read_array(psa: *mut SAFEARRAY) -> Vec<i64> {
    let mut out = Vec::new();
    if psa.is_null() {
        return out;
    }
    let (Ok(lower), Ok(upper), Ok(vt)) = (SafeArrayGetLBound(psa, 1), SafeArrayGetUBound(psa, 1), SafeArrayGetVartype(psa)) else {
        return out;
    };
    let count = (upper - lower + 1).max(0) as usize;
    let mut data: *mut core::ffi::c_void = std::ptr::null_mut();
    if SafeArrayAccessData(psa, &mut data).is_err() {
        return out;
    }
    for i in 0..count {
        let v = match vt {
            VT_UI1 => *(data as *const u8).add(i) as i64,
            VT_I1 => *(data as *const i8).add(i) as i64,
            VT_I2 => *(data as *const i16).add(i) as i64,
            VT_UI2 => *(data as *const u16).add(i) as i64,
            VT_I4 | VT_INT => *(data as *const i32).add(i) as i64,
            VT_UI4 | VT_UINT => *(data as *const u32).add(i) as i64,
            VT_I8 => *(data as *const i64).add(i),
            VT_UI8 => *(data as *const u64).add(i) as i64,
            _ => 0,
        };
        out.push(v);
    }
    let _ = SafeArrayUnaccessData(psa);
    out
}

unsafe fn from_variant(v: &VARIANT) -> Val {
    let inner = &v.Anonymous.Anonymous;
    let vt = inner.vt;
    if vt == VT_EMPTY || vt == VT_NULL {
        return Val::Null;
    }
    if vt.0 & VT_ARRAY.0 != 0 {
        return Val::Array(read_array(inner.Anonymous.parray));
    }
    if vt == VT_BSTR {
        return Val::Str(inner.Anonymous.bstrVal.to_string());
    }
    if vt == VT_BOOL {
        return Val::Bool(inner.Anonymous.boolVal.0 != 0);
    }
    match scalar(vt, &inner.Anonymous) {
        Some(i) => Val::Int(i),
        None => Val::Null,
    }
}

/// Wrapper around a WMI object for reading properties
pub struct Obj(IWbemClassObject);

impl Obj {
    pub fn get(&self, name: &str) -> Val {
        unsafe {
            let w = wide(name);
            let mut v = VARIANT::default();
            if self.0.Get(PCWSTR(w.as_ptr()), 0, &mut v, None, None).is_err() {
                return Val::Null;
            }
            let val = from_variant(&v);
            let _ = VariantClear(&mut v);
            val
        }
    }
    pub fn i32(&self, name: &str) -> Option<i32> {
        self.get(name).as_i32()
    }
}

/// Method argument
#[derive(Debug, Clone)]
pub enum Arg {
    I32(i32),
    U32(u32),
    U8(u8),
    Bytes(Vec<u8>),
}

unsafe fn to_variant(arg: &Arg) -> VARIANT {
    let mut v = VARIANT::default();
    let inner = &mut *v.Anonymous.Anonymous;
    match arg {
        Arg::I32(x) => {
            inner.vt = VT_I4;
            inner.Anonymous.lVal = *x;
        }
        Arg::U32(x) => {
            // WMI accepts uint32 parameters as VT_I4
            inner.vt = VT_I4;
            inner.Anonymous.lVal = *x as i32;
        }
        Arg::U8(x) => {
            inner.vt = VT_UI1;
            inner.Anonymous.bVal = *x;
        }
        Arg::Bytes(bytes) => {
            let psa = SafeArrayCreateVector(VT_UI1, 0, bytes.len() as u32);
            let mut data: *mut core::ffi::c_void = std::ptr::null_mut();
            if SafeArrayAccessData(psa, &mut data).is_ok() {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), data as *mut u8, bytes.len());
                let _ = SafeArrayUnaccessData(psa);
            }
            inner.vt = VARENUM(VT_ARRAY.0 | VT_UI1.0);
            inner.Anonymous.parray = psa;
        }
    }
    v
}

// ---------------- operations ----------------

fn exec_query(services: &IWbemServices, query: &str) -> Result<Vec<IWbemClassObject>> {
    unsafe {
        let enumerator = services
            .ExecQuery(&BSTR::from("WQL"), &BSTR::from(query), WBEM_GENERIC_FLAG_TYPE(WBEM_FLAG_RETURN_IMMEDIATELY.0 | WBEM_FLAG_FORWARD_ONLY.0), None)
            .map_err(|e| format!("{query}: {e}"))?;
        let mut out = Vec::new();
        loop {
            let mut objs = [None];
            let mut returned = 0u32;
            let hr = enumerator.Next(WBEM_INFINITE, &mut objs, &mut returned);
            if hr.is_err() || returned == 0 {
                break;
            }
            if let Some(o) = objs[0].take() {
                out.push(o);
            }
        }
        Ok(out)
    }
}

pub fn query(query: &str) -> Result<Vec<Obj>> {
    with_conn(|c| Ok(exec_query(&c.services, query)?.into_iter().map(Obj).collect()))
}

/// One-off query in another namespace (e.g. root\cimv2), on its own connection
pub fn query_in(namespace: &str, query: &str) -> Result<Vec<Obj>> {
    let services = connect_to(namespace)?;
    Ok(exec_query(&services, query)?.into_iter().map(Obj).collect())
}

pub fn exists(class: &str) -> bool {
    instance_path(class).map(|p| p.is_some()).unwrap_or(false)
}

/// __PATH of the first instance of a class (cached per thread)
fn instance_path(class: &str) -> Result<Option<String>> {
    with_conn(|c| {
        if let Some(p) = c.paths.get(class) {
            return Ok(p.clone());
        }
        let objs = exec_query(&c.services, &format!("SELECT * FROM {class}"))?;
        let path = objs.first().and_then(|o| Obj(o.clone()).get("__PATH").as_str().map(|s| s.to_string()));
        c.paths.insert(class.to_string(), path.clone());
        Ok(path)
    })
}

/// Calls a method on the first instance of `class`; returns the out-parameters object,
/// or None for methods without outputs (most Lenovo setters: SetSmartFanMode, SetFeatureValue, ...)
pub fn call(class: &str, method: &str, args: &[(&str, Arg)]) -> Result<Option<Obj>> {
    let path = instance_path(class)?.ok_or_else(|| format!("{class} not available"))?;
    with_conn(|c| unsafe {
        let mut class_obj: Option<IWbemClassObject> = None;
        c.services
            .GetObject(&BSTR::from(class), WBEM_GENERIC_FLAG_TYPE(0), None, Some(&mut class_obj), None)
            .map_err(|e| format!("GetObject {class}: {e}"))?;
        let class_obj = class_obj.ok_or("no class object")?;

        let wmethod = wide(method);
        let mut in_sig: Option<IWbemClassObject> = None;
        let mut out_sig: Option<IWbemClassObject> = None;
        class_obj
            .GetMethod(PCWSTR(wmethod.as_ptr()), 0, &mut in_sig, &mut out_sig)
            .map_err(|e| format!("GetMethod {class}.{method}: {e}"))?;

        let in_params = match in_sig {
            Some(sig) => {
                let inst = sig.SpawnInstance(0).map_err(|e| e.to_string())?;
                for (name, arg) in args {
                    let w = wide(name);
                    let mut v = to_variant(arg);
                    let r = inst.Put(PCWSTR(w.as_ptr()), 0, &v, 0);
                    let _ = VariantClear(&mut v);
                    r.map_err(|e| format!("Put {name}: {e}"))?;
                }
                Some(inst)
            }
            None => None,
        };

        let mut out: Option<IWbemClassObject> = None;
        c.services
            .ExecMethod(&BSTR::from(path.as_str()), &BSTR::from(method), WBEM_GENERIC_FLAG_TYPE(0), None, in_params.as_ref(), Some(&mut out), None)
            .map_err(|e| format!("{class}.{method}: {e}"))?;
        Ok(out.map(Obj))
    })
}

/// Calls a method and reads one integer out-parameter
pub fn call_int(class: &str, method: &str, out_name: &str, args: &[(&str, Arg)]) -> Result<i32> {
    call(class, method, args)?.and_then(|o| o.i32(out_name)).ok_or_else(|| format!("{class}.{method}: no {out_name}"))
}

/// Same as call_int but logs failures and returns None
pub fn try_call_int(class: &str, method: &str, out_name: &str, args: &[(&str, Arg)], log: bool) -> Option<i32> {
    match call_int(class, method, out_name, args) {
        Ok(v) => Some(v),
        Err(e) => {
            if log {
                crate::log!("WMI {e}");
            }
            None
        }
    }
}

/// Subscribes to an extrinsic event class on a background thread. Drop the returned handle to stop.
pub struct Listener(Arc<AtomicBool>);

impl Drop for Listener {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

pub fn listen(class: &str, handler: impl Fn(Obj) + Send + 'static) -> Listener {
    let running = Arc::new(AtomicBool::new(true));
    let flag = running.clone();
    let class = class.to_string();
    std::thread::Builder::new()
        .name(format!("wmi-{class}"))
        .spawn(move || {
            let run = || -> Result<()> {
                let services = connect()?;
                let enumerator = unsafe {
                    services
                        .ExecNotificationQuery(&BSTR::from("WQL"), &BSTR::from(format!("SELECT * FROM {class}")), WBEM_GENERIC_FLAG_TYPE(WBEM_FLAG_RETURN_IMMEDIATELY.0 | WBEM_FLAG_FORWARD_ONLY.0), None)
                        .map_err(|e| format!("subscribe {class}: {e}"))?
                };
                while flag.load(Ordering::SeqCst) {
                    let mut objs = [None];
                    let mut returned = 0u32;
                    let _ = unsafe { enumerator.Next(500, &mut objs, &mut returned) };
                    if returned > 0 {
                        if let Some(o) = objs[0].take() {
                            handler(Obj(o));
                        }
                    }
                }
                Ok(())
            };
            if let Err(e) = run() {
                crate::log!("Can't listen to {class}: {e}");
            }
        })
        .ok();
    Listener(running)
}
