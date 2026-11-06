//! X11：系统输入法（XIM）崩溃/重启后，让本程序重新接上它。
//!
//! 解决的现象：输入法（fcitx + 搜狗）崩溃/重启之后，本程序打英文正常，但
//! `Ctrl+Space` 再也切不出中文，只能重启程序。
//!
//! # 事实（libX11 1.8.x 源码 + 本机探针实测）
//!
//! * winit 用 XIM 调系统输入法（`XMODIFIERS=@im=fcitx`）。IM 打开成功后，libX11 的
//!   `_XimRegisterServerFilter()` 会以 **本连接** 对输入法 XIM 服务器窗口
//!   （`XIM_SERVERS` 里 `@server=<名字>` 那个 atom 的 selection owner）调用
//!   `XSelectInput(..., StructureNotifyMask)`，用于收到服务器销毁事件
//!   → `_XimFilterServerDestroy()` → `_XimServerDestroy()`
//!   → winit 注册的 `XNDestroyCallback`（`xim_destroy_callback`：注册
//!   IM-instantiate 回调 + `replace_im()` 重开 IM）。
//! * 这条链是 winit 唯一的自愈入口，而且它**会被永久废掉**：
//!   `xim_destroy_callback` 只在 `!is_fallback` 时注册 instantiate 回调；一旦
//!   某次崩溃后 `replace_im()` 退到了 `@im=local` 兜底（`is_fallback = true`），
//!   之后再崩就什么都不做、也不会再注册重连回调 —— 于是输入法回来也永远接不上。
//!   实机日志证据：`X11 error: BadWindow, request_code: 18/25`（X_ChangeProperty /
//!   X_SendEvent，即 XIM 在往已销毁的服务器窗口发消息）。
//! * 光合成 `XIM_SERVERS` 的 PropertyNotify 没用：libX11 `_XimFilterPropertyNotify()`
//!   只对 `!icb->call` 的待触发回调有效，回调没注册/已触发过就完全空转。
//! * 反过来，合成 `DestroyNotify` 打中输入法服务器窗口则必定命中
//!   （`_XimFilterServerDestroy` 只看事件类型），能让 winit 走它本该走的销毁→重开路径。
//!
//! # 做法
//!
//! 每次按键 / 窗口获得焦点时（节流 250ms）检查一次：
//!
//! 1. 输入法服务器窗口消失了（输入法刚挂）：对它补发 `DestroyNotify`。这样 winit
//!    立刻走销毁路径 —— 在 `is_fallback` 还是 false 的时候把重连回调装上，退到
//!    local 兜底后仍然可以恢复；否则就进入“永久失效”状态。
//! 2. 服务器窗口换了（输入法重启到了新窗口）：先对旧窗口补发 `DestroyNotify`
//!    拆掉已失效的连接，再补发 `XIM_SERVERS` PropertyNotify 走 instantiate 路径，
//!    让 winit 用**当前**的输入法重开。
//! 3. 用 `XGetWindowAttributes().your_event_mask` 判断“本进程是否还连在这个服务器
//!    窗口上”（libX11 打上的正是 `StructureNotifyMask`）；没连上就持续重试第 2 步。
//! 4. 重开 IM 后新 IC 没有被 `XSetICFocus`（winit 只在 XI2 `FocusIn` 时设置），
//!    输入法会认为该客户端未激活、仍不处理转发按键，所以最后做一次焦点往返，
//!    让 winit 对当前 IC 重新 `XSetICFocus`。
//!
//! 前提：只在“本程序用的那个输入法服务器窗口确实消失/换了”时才合成销毁。
//! 实测在输入法仍活着时伪造销毁，会让 fcitx 拒绝同一客户端再次 `XOpenIM()`，
//! 反而把好的连接弄坏。
//!
//! # 已知限制
//!
//! * 应用启动时输入法就已经落在 `@im=local` 兜底、且没有待触发的 instantiate
//!   回调时，本模块无法凭空恢复。
//! * 对已失效 atom 调用 `XGetAtomName` 仍可能产生 Xlib 异步 `BadAtom` 日志。

#[cfg(target_os = "linux")]
use std::os::raw::{c_char, c_int, c_long, c_ulong, c_void};
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
#[cfg(target_os = "linux")]
use std::sync::OnceLock;
#[cfg(target_os = "linux")]
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
use wgpu::rwh::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};

/// 两次检查之间的最小间隔：按键很密集时不必每次都查属性。
#[cfg(target_os = "linux")]
const MIN_INTERVAL: Duration = Duration::from_millis(250);

/// 连续恢复尝试上限（约 12 秒 @300ms）：输入法长期不可用时停止定时轮询，
/// 避免事件循环反复唤醒与日志刷屏；按键/焦点事件仍会重新触发检查。
#[cfg(target_os = "linux")]
const MAX_RECOVERY_ATTEMPTS: u32 = 40;

/// `DestroyNotify`（X11 核心协议事件号）。
#[cfg(target_os = "linux")]
const DESTROY_NOTIFY: c_int = 17;

/// `PropertyNotify`。
#[cfg(target_os = "linux")]
const PROPERTY_NOTIFY: c_int = 28;

/// `PropertyNewValue`。
#[cfg(target_os = "linux")]
const PROPERTY_NEW_VALUE: c_int = 0;

/// `XA_ATOM`。
#[cfg(target_os = "linux")]
const XA_ATOM: c_ulong = 4;

/// `Success`。
#[cfg(target_os = "linux")]
const SUCCESS: c_int = 0;

/// `StructureNotifyMask`。
#[cfg(target_os = "linux")]
const STRUCTURE_NOTIFY_MASK: c_long = 1 << 17;

/// `RevertToParent`。
#[cfg(target_os = "linux")]
const REVERT_TO_PARENT: c_int = 2;

/// `CurrentTime`。
#[cfg(target_os = "linux")]
const CURRENT_TIME: c_ulong = 0;

/// 传给 `XFilterEvent` 的缓冲区大小。XEvent 在 64 位上是 192 字节，这里多留余量。
#[cfg(target_os = "linux")]
const XEVENT_BUFFER: usize = 256;

/// `XEvent` 要求 8 字节对齐；裸 `u8` 数组不能依赖栈上的偶然对齐。
#[cfg(target_os = "linux")]
#[repr(C, align(16))]
struct XEventBuffer([u8; XEVENT_BUFFER]);

#[cfg(target_os = "linux")]
impl XEventBuffer {
    fn zeroed() -> Self {
        Self([0u8; XEVENT_BUFFER])
    }
}

/// `XPropertyEvent`（libX11 布局，64 位）。
#[cfg(target_os = "linux")]
#[repr(C)]
struct XPropertyEvent {
    type_: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut c_void,
    window: c_ulong,
    atom: c_ulong,
    time: c_ulong,
    state: c_int,
}

/// `XDestroyWindowEvent`（libX11 布局，64 位）。
#[cfg(target_os = "linux")]
#[repr(C)]
struct XDestroyWindowEvent {
    type_: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut c_void,
    event: c_ulong,
    window: c_ulong,
}

/// `XWindowAttributes`（libX11 布局；只关心 `your_event_mask`）。
#[cfg(target_os = "linux")]
#[repr(C)]
struct XWindowAttributes {
    x: c_int,
    y: c_int,
    width: c_int,
    height: c_int,
    border_width: c_int,
    depth: c_int,
    visual: *mut c_void,
    root: c_ulong,
    class: c_int,
    bit_gravity: c_int,
    win_gravity: c_int,
    backing_store: c_int,
    backing_planes: c_ulong,
    backing_pixel: c_ulong,
    save_under: c_int,
    colormap: c_ulong,
    map_installed: c_int,
    map_state: c_int,
    all_event_masks: c_long,
    your_event_mask: c_long,
    do_not_propagate_mask: c_long,
    override_redirect: c_int,
    screen: *mut c_void,
}

impl XWindowAttributes {
    fn empty() -> Self {
        // SAFETY: 全零是合法的 POD 初值；Xlib 会完整填充。
        unsafe { std::mem::zeroed() }
    }
}

/// 需要的最小 libX11 入口（dlopen 加载，不新增编译期依赖）。
#[cfg(target_os = "linux")]
struct Xlib {
    filter_event: unsafe extern "C" fn(*mut c_void, c_ulong) -> c_int,
    default_root_window: unsafe extern "C" fn(*mut c_void) -> c_ulong,
    intern_atom: unsafe extern "C" fn(*mut c_void, *const c_char, c_int) -> c_ulong,
    get_window_property: unsafe extern "C" fn(
        *mut c_void,
        c_ulong,
        c_ulong,
        c_long,
        c_long,
        c_int,
        c_ulong,
        *mut c_ulong,
        *mut c_int,
        *mut c_ulong,
        *mut c_ulong,
        *mut *mut u8,
    ) -> c_int,
    get_window_attributes:
        unsafe extern "C" fn(*mut c_void, c_ulong, *mut XWindowAttributes) -> c_int,
    get_selection_owner: unsafe extern "C" fn(*mut c_void, c_ulong) -> c_ulong,
    get_atom_name: unsafe extern "C" fn(*mut c_void, c_ulong) -> *mut c_char,
    free: unsafe extern "C" fn(*mut c_void),
    get_input_focus: unsafe extern "C" fn(*mut c_void, *mut c_ulong, *mut c_int) -> c_int,
    set_input_focus: unsafe extern "C" fn(*mut c_void, c_ulong, c_int, c_ulong) -> c_int,
    sync: unsafe extern "C" fn(*mut c_void, c_int) -> c_int,
}

/// 把 `dlsym` 拿到的裸指针转成固定签名的函数指针（大小相同，调用方已校验非空）。
#[cfg(target_os = "linux")]
unsafe fn cast_symbol<T: Copy>(pointer: *mut c_void) -> T {
    debug_assert_eq!(std::mem::size_of::<T>(), std::mem::size_of::<*mut c_void>());
    unsafe { std::mem::transmute_copy(&pointer) }
}

#[cfg(target_os = "linux")]
fn xlib() -> Option<&'static Xlib> {
    static LIB: OnceLock<Option<Xlib>> = OnceLock::new();
    LIB.get_or_init(|| {
        // SAFETY: 只加载本进程本来就依赖的 libX11，并解析固定签名的符号。
        unsafe {
            let handle = libc::dlopen(c"libX11.so.6".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
            if handle.is_null() {
                log::debug!("[IME] 无法加载 libX11，跳过 XIM 补发");
                return None;
            }
            let symbol = |name: &std::ffi::CStr| libc::dlsym(handle, name.as_ptr());
            let filter_event = symbol(c"XFilterEvent");
            let default_root_window = symbol(c"XDefaultRootWindow");
            let intern_atom = symbol(c"XInternAtom");
            let get_window_property = symbol(c"XGetWindowProperty");
            let get_window_attributes = symbol(c"XGetWindowAttributes");
            let get_selection_owner = symbol(c"XGetSelectionOwner");
            let get_atom_name = symbol(c"XGetAtomName");
            let free = symbol(c"XFree");
            let get_input_focus = symbol(c"XGetInputFocus");
            let set_input_focus = symbol(c"XSetInputFocus");
            let sync = symbol(c"XSync");
            if [
                filter_event,
                default_root_window,
                intern_atom,
                get_window_property,
                get_window_attributes,
                get_selection_owner,
                get_atom_name,
                free,
                get_input_focus,
                set_input_focus,
                sync,
            ]
            .iter()
            .any(|symbol| symbol.is_null())
            {
                log::debug!("[IME] libX11 缺少所需符号，跳过 XIM 补发");
                return None;
            }
            Some(Xlib {
                filter_event: cast_symbol(filter_event),
                default_root_window: cast_symbol(default_root_window),
                intern_atom: cast_symbol(intern_atom),
                get_window_property: cast_symbol(get_window_property),
                get_window_attributes: cast_symbol(get_window_attributes),
                get_selection_owner: cast_symbol(get_selection_owner),
                get_atom_name: cast_symbol(get_atom_name),
                free: cast_symbol(free),
                get_input_focus: cast_symbol(get_input_focus),
                set_input_focus: cast_symbol(set_input_focus),
                sync: cast_symbol(sync),
            })
        }
    })
    .as_ref()
}

/// 节流用的时间基准与上一次检查时刻（毫秒）。
#[cfg(target_os = "linux")]
static EPOCH: OnceLock<Instant> = OnceLock::new();
#[cfg(target_os = "linux")]
static LAST_CHECK_MS: AtomicU64 = AtomicU64::new(0);

/// 最近一次看到的“本程序所用输入法的 XIM 服务器窗口”。0 表示还没见过可用的服务器。
#[cfg(target_os = "linux")]
static LAST_SERVER_WINDOW: AtomicU64 = AtomicU64::new(0);

/// 最近一次确认“本进程还连在这个服务器窗口上”的窗口；0 表示未连接。
#[cfg(target_os = "linux")]
static ATTACHED_SERVER_WINDOW: AtomicU64 = AtomicU64::new(0);

/// 是否处于“输入法在、但本进程还没连上”的待恢复状态（用于触发短间隔轮询）。
#[cfg(target_os = "linux")]
static NEEDS_ATTENTION: AtomicBool = AtomicBool::new(false);

/// 当前恢复周期内已尝试的次数，用于停止无休止的轮询。
#[cfg(target_os = "linux")]
static RECOVERY_ATTEMPTS: AtomicU32 = AtomicU32::new(0);

/// 从 `XMODIFIERS`（例如 `@im=fcitx`）取出输入法名字，用作匹配 `@server=<名字>`。
#[cfg(target_os = "linux")]
fn im_name() -> &'static str {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        let raw = std::env::var("XMODIFIERS").unwrap_or_default();
        let Some(rest) = raw.split("@im=").nth(1) else {
            return String::new();
        };
        rest.split([':', ',', '/'])
            .next()
            .unwrap_or_default()
            .trim()
            .to_string()
    })
}

/// 取 winit 用于 XIM 的那条 Xlib 连接（非 X11 会话返回 `None`）。
#[cfg(target_os = "linux")]
fn winit_xlib_display(window: &winit::window::Window) -> Option<std::ptr::NonNull<c_void>> {
    let handle = window.display_handle().ok()?;
    match handle.as_raw() {
        RawDisplayHandle::Xlib(xlib) => xlib.display,
        _ => None,
    }
}

/// 取本窗口的 X11 window id。
#[cfg(target_os = "linux")]
fn window_xid(window: &winit::window::Window) -> Option<c_ulong> {
    let handle = window.window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::Xlib(xlib) => Some(xlib.window as c_ulong),
        _ => None,
    }
}

/// 读 `XIM_SERVERS`，找出本程序所用输入法（`@server=<名字>`）的 XIM 服务器窗口。
#[cfg(target_os = "linux")]
unsafe fn current_server_window(xlib: &Xlib, display: *mut c_void, name: &str) -> c_ulong {
    if name.is_empty() {
        return 0;
    }
    let root = unsafe { (xlib.default_root_window)(display) };
    if root == 0 {
        return 0;
    }
    let atom_servers = unsafe { (xlib.intern_atom)(display, c"XIM_SERVERS".as_ptr(), 1) };
    if atom_servers == 0 {
        return 0;
    }

    let mut actual_type: c_ulong = 0;
    let mut actual_format: c_int = 0;
    let mut nitems: c_ulong = 0;
    let mut bytes_after: c_ulong = 0;
    let mut prop: *mut u8 = std::ptr::null_mut();
    let status = unsafe {
        (xlib.get_window_property)(
            display,
            root,
            atom_servers,
            0,
            1024,
            0,
            XA_ATOM,
            &mut actual_type,
            &mut actual_format,
            &mut nitems,
            &mut bytes_after,
            &mut prop,
        )
    };
    if prop.is_null() {
        return 0;
    }
    // XIM_SERVERS 必须是 32 位 ATOM 数组；格式不符时按不可用处理，
    // 否则下面按 c_ulong 步长遍历会越界读。
    if status != SUCCESS || actual_type != XA_ATOM || actual_format != 32 || nitems > 1024 {
        unsafe { (xlib.free)(prop.cast()) };
        return 0;
    }

    let mut found = 0;
    for index in 0..nitems as usize {
        let atom = unsafe { *(prop as *const c_ulong).add(index) };
        if atom == 0 {
            // 无效 atom 会触发 Xlib 异步 BadAtom。
            continue;
        }
        let atom_name = unsafe { (xlib.get_atom_name)(display, atom) };
        if atom_name.is_null() {
            continue;
        }
        let text = unsafe { std::ffi::CStr::from_ptr(atom_name) }
            .to_string_lossy()
            .to_string();
        unsafe { (xlib.free)(atom_name.cast()) };
        let matched = text
            .strip_prefix("@server=")
            .or_else(|| text.strip_prefix("@im="))
            .is_some_and(|server| server == name);
        if matched {
            let owner = unsafe { (xlib.get_selection_owner)(display, atom) };
            if owner != 0 {
                found = owner;
            }
        }
    }
    unsafe { (xlib.free)(prop.cast()) };
    found
}

/// 本进程是否还连在这个输入法服务器窗口上（libX11 打的是 `StructureNotifyMask`）。
#[cfg(target_os = "linux")]
unsafe fn is_attached(xlib: &Xlib, display: *mut c_void, server_window: c_ulong) -> bool {
    if server_window == 0 {
        return false;
    }
    let mut attributes = XWindowAttributes::empty();
    let status = unsafe { (xlib.get_window_attributes)(display, server_window, &mut attributes) };
    status != 0 && (attributes.your_event_mask & STRUCTURE_NOTIFY_MASK) != 0
}

/// 对输入法服务器窗口合成一次 `DestroyNotify`，让 libX11 走它的销毁路径。
#[cfg(target_os = "linux")]
unsafe fn dispatch_server_destroy(
    xlib: &Xlib,
    display: *mut c_void,
    server_window: c_ulong,
) -> bool {
    if server_window == 0 {
        return false;
    }
    let event = XDestroyWindowEvent {
        type_: DESTROY_NOTIFY,
        serial: 0,
        send_event: 0,
        display,
        event: server_window,
        window: server_window,
    };
    let mut buffer = XEventBuffer::zeroed();
    unsafe {
        std::ptr::copy_nonoverlapping(
            (&event as *const XDestroyWindowEvent).cast::<u8>(),
            buffer.0.as_mut_ptr(),
            std::mem::size_of::<XDestroyWindowEvent>(),
        );
        (xlib.filter_event)(buffer.0.as_mut_ptr().cast::<c_void>(), server_window) != 0
    }
}

/// 合成 `XIM_SERVERS` 的 PropertyNotify（覆盖 instantiate 回调路径）。
#[cfg(target_os = "linux")]
unsafe fn dispatch_xim_servers_notify(xlib: &Xlib, display: *mut c_void) -> bool {
    let root = unsafe { (xlib.default_root_window)(display) };
    if root == 0 {
        return false;
    }
    let atom = unsafe { (xlib.intern_atom)(display, c"XIM_SERVERS".as_ptr(), 0) };
    if atom == 0 {
        return false;
    }
    let event = XPropertyEvent {
        type_: PROPERTY_NOTIFY,
        serial: 0,
        send_event: 0,
        display,
        window: root,
        atom,
        time: 0,
        state: PROPERTY_NEW_VALUE,
    };
    let mut buffer = XEventBuffer::zeroed();
    unsafe {
        std::ptr::copy_nonoverlapping(
            (&event as *const XPropertyEvent).cast::<u8>(),
            buffer.0.as_mut_ptr(),
            std::mem::size_of::<XPropertyEvent>(),
        );
        (xlib.filter_event)(buffer.0.as_mut_ptr().cast::<c_void>(), root) != 0
    }
}

/// 重开 IM 后新 IC 没有被 `XSetICFocus`：做一次焦点往返，让 winit 对当前 IC 重新设焦点。
///
/// 只在焦点确实已经在本窗口时执行；直接对同一窗口重复 `XSetInputFocus` 不会产生事件，
/// 所以要先切到 root 再切回来。
#[cfg(target_os = "linux")]
unsafe fn nudge_ic_focus(xlib: &Xlib, display: *mut c_void, window: c_ulong) {
    let mut current: c_ulong = 0;
    let mut revert: c_int = 0;
    unsafe { (xlib.get_input_focus)(display, &mut current, &mut revert) };
    if current != window {
        return;
    }
    let root = unsafe { (xlib.default_root_window)(display) };
    if root == 0 {
        return;
    }
    unsafe {
        (xlib.set_input_focus)(display, root, REVERT_TO_PARENT, CURRENT_TIME);
        (xlib.sync)(display, 0);
        (xlib.set_input_focus)(display, window, REVERT_TO_PARENT, CURRENT_TIME);
        (xlib.sync)(display, 0);
    }
}

/// 在按键/获得焦点等时机检查输入法状态，并在需要时把本程序重新接上。
#[cfg(target_os = "linux")]
pub(super) fn dispatch_pending(window: &winit::window::Window) {
    let now_ms = EPOCH.get_or_init(Instant::now).elapsed().as_millis() as u64;
    let last_ms = LAST_CHECK_MS.load(Ordering::Relaxed);
    if now_ms.saturating_sub(last_ms) < MIN_INTERVAL.as_millis() as u64 {
        return;
    }
    LAST_CHECK_MS.store(now_ms, Ordering::Relaxed);

    let Some(display) = winit_xlib_display(window) else {
        return;
    };
    let Some(xlib) = xlib() else {
        return;
    };

    // SAFETY: display 来自 winit 的 XlibDisplayHandle（同一线程内使用）。
    unsafe {
        let display = display.as_ptr();
        let last = LAST_SERVER_WINDOW.load(Ordering::Relaxed);
        let server = current_server_window(xlib, display, im_name());
        let attached = if server != 0 {
            is_attached(xlib, display, server)
        } else {
            false
        };
        log::debug!("[IME] 检查：server={server:#x} last={last:#x} attached={attached}");

        if attached {
            LAST_SERVER_WINDOW.store(server, Ordering::Relaxed);
            ATTACHED_SERVER_WINDOW.store(server, Ordering::Relaxed);
            NEEDS_ATTENTION.store(false, Ordering::Relaxed);
            RECOVERY_ATTEMPTS.store(0, Ordering::Relaxed);
            return;
        }

        if server == 0 {
            // 输入法刚挂掉：抢在 winit 落到 local 兜底之前补一发销毁。
            // 这样 winit 的 destroy 回调会注册重连回调（is_fallback 仍为 false），
            // 输入法回来时才有机会自动接上。
            let previous = ATTACHED_SERVER_WINDOW.swap(0, Ordering::Relaxed);
            LAST_SERVER_WINDOW.store(0, Ordering::Relaxed);
            RECOVERY_ATTEMPTS.store(0, Ordering::Relaxed);
            if previous != 0 {
                let hit = dispatch_server_destroy(xlib, display, previous);
                log::warn!("[IME] 输入法已消失，已补发 XIM 销毁事件（命中={hit}）");
            }
            NEEDS_ATTENTION.store(false, Ordering::Relaxed);
            return;
        }

        // 输入法在、但我们没连上：可能是刚崩溃重启，也可能是之前掉进了兜底 IM。
        // 换了服务器窗口说明是真实重启，重新给满一轮尝试；
        // 连续尝试达到上限后停止定时轮询，避免输入法长期不可用时反复唤醒和刷日志。
        let server_changed = last != server;
        if server_changed {
            RECOVERY_ATTEMPTS.store(0, Ordering::Relaxed);
        }
        let attempts = RECOVERY_ATTEMPTS
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        NEEDS_ATTENTION.store(attempts <= MAX_RECOVERY_ATTEMPTS, Ordering::Relaxed);
        let previous = ATTACHED_SERVER_WINDOW.swap(0, Ordering::Relaxed);
        if previous != 0 {
            dispatch_server_destroy(xlib, display, previous);
        }
        let notify_hit = dispatch_xim_servers_notify(xlib, display);
        if server_changed || previous != 0 {
            log::warn!(
                "[IME] 触发 XIM 重连（server={server:#x}，尝试 {attempts}/{MAX_RECOVERY_ATTEMPTS}，命中={notify_hit}）"
            );
        }
        LAST_SERVER_WINDOW.store(server, Ordering::Relaxed);

        // 只有真正的状态迁移（销毁了旧连接 / 换了服务器）才做焦点往返，
        // 否则每一轮检查都会让焦点在 root 与窗口之间抖动。
        if server_changed || previous != 0 {
            if let Some(xid) = window_xid(window) {
                nudge_ic_focus(xlib, display, xid);
            }
        }
    }
}

/// 是否还在等待输入法重连（供事件循环安排短间隔轮询）。
#[cfg(target_os = "linux")]
pub(super) fn needs_attention() -> bool {
    NEEDS_ATTENTION.load(Ordering::Relaxed)
}

/// 非 Linux 平台：不做任何事（XIM 仅存在于 X11）。
#[cfg(not(target_os = "linux"))]
pub(super) fn dispatch_pending(_window: &winit::window::Window) {}

/// 非 Linux 平台：永远不需要额外轮询。
#[cfg(not(target_os = "linux"))]
pub(super) fn needs_attention() -> bool {
    false
}
