//! The Vulkan implicit layer `VK_LAYER_KETTLE_overlay`: draws the overlay into the swapchain image just before
//! the game's `vkQueuePresentKHR`. Works for native Vulkan games and for DXVK and VKD3D (Proton), which are
//! ordinary Vulkan applications.
//!
//! How a frame is added: the layer submits one command buffer on the present queue that waits for the
//! semaphores the game's present waits for, draws egui into the image (a render pass that loads what the game
//! drew), and signals its own semaphore; the present then waits for that one only. A ring of three frames of
//! resources (command buffer, fence, semaphore, vertex and index memory) is reused once its fence is done.

use crate::state::{self, Claim};
use ash::vk::{self, Handle};
use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_void};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

const FRAMES: usize = 3;
const FENCE_WAIT_NS: u64 = 2_000_000_000;

// ---- the loader's layer interface --------------------------------------------------------------------------

#[repr(C)]
struct InstanceLink {
    next: *mut InstanceLink,
    gipa: vk::PFN_vkGetInstanceProcAddr,
    gpdpa: *const c_void,
}

#[repr(C)]
struct DeviceLink {
    next: *mut DeviceLink,
    gipa: vk::PFN_vkGetInstanceProcAddr,
    gdpa: vk::PFN_vkGetDeviceProcAddr,
}

#[repr(C)]
struct LayerCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    function: i32,
    /// A union of pointers in the loader's header: the link, or a callback.
    info: *mut c_void,
}

const LOADER_INSTANCE_CREATE_INFO: i32 = 47;
const LOADER_DEVICE_CREATE_INFO: i32 = 48;
const LAYER_LINK_INFO: i32 = 0;
const LOADER_DATA_CALLBACK: i32 = 1;

#[repr(C)]
pub struct NegotiateInterface {
    s_type: u32,
    p_next: *mut c_void,
    version: u32,
    gipa: Option<unsafe extern "system" fn(vk::Instance, *const c_char) -> vk::PFN_vkVoidFunction>,
    gdpa: Option<unsafe extern "system" fn(vk::Device, *const c_char) -> vk::PFN_vkVoidFunction>,
    gpdpa: *const c_void,
}

unsafe fn find_create_info(mut p: *const c_void, ty: i32, function: i32) -> *mut LayerCreateInfo {
    while !p.is_null() {
        let base = p as *const vk::BaseInStructure;
        let ci = p as *mut LayerCreateInfo;
        if unsafe { (*ci).s_type == ty && (*ci).function == function } {
            return ci;
        }
        p = unsafe { (*base).p_next as *const c_void };
    }
    std::ptr::null_mut()
}

/// The loader's dispatch table pointer, the first word of every dispatchable handle: a key for lookups.
unsafe fn key_of(handle: u64) -> usize {
    if handle == 0 { 0 } else { unsafe { *(handle as *const usize) } }
}

// ---- what we know about the game's Vulkan objects ----------------------------------------------------------

#[allow(dead_code)]
struct Inst {
    handle: vk::Instance,
    ash: ash::Instance,
    gipa: vk::PFN_vkGetInstanceProcAddr,
}

type SetLoaderData = unsafe extern "system" fn(vk::Device, *mut c_void) -> vk::Result;

#[allow(dead_code)]
struct Dev {
    handle: vk::Device,
    phys: vk::PhysicalDevice,
    ash: ash::Device,
    swapchain: ash::khr::swapchain::DeviceFn,
    inst: Arc<Inst>,
    gdpa: vk::PFN_vkGetDeviceProcAddr,
    set_loader_data: Option<SetLoaderData>,
    families: Vec<vk::QueueFamilyProperties>,
    mem: vk::PhysicalDeviceMemoryProperties,
    queues: Mutex<HashMap<u64, u32>>,
    swaps: Mutex<HashMap<u64, Swap>>,
    renderer: Mutex<Option<Renderer>>,
}

struct Swap {
    format: vk::Format,
    extent: vk::Extent2D,
    /// Created with the colour-attachment usage we need and a format we can draw to.
    usable: bool,
    /// Created with the transfer-source usage too: a screenshot can copy its images.
    readable: bool,
    images: Vec<vk::Image>,
    views: Vec<vk::ImageView>,
    fbs: Vec<vk::Framebuffer>,
    pass: vk::RenderPass,
}

#[derive(Default)]
struct Registry {
    instances: HashMap<usize, Arc<Inst>>,
    devices: HashMap<usize, Arc<Dev>>,
    /// The next layer's lookup functions by dispatch key, recorded first and by plain inserts only: whatever
    /// goes wrong later in our bookkeeping, the game's own lookups still reach the driver.
    gipas: HashMap<usize, vk::PFN_vkGetInstanceProcAddr>,
    gdpas: HashMap<usize, vk::PFN_vkGetDeviceProcAddr>,
}

fn reg() -> MutexGuard<'static, Registry> {
    static R: OnceLock<Mutex<Registry>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(Registry::default())).lock().unwrap_or_else(|e| e.into_inner())
}

fn inst_of(handle: u64) -> Option<Arc<Inst>> {
    let k = unsafe { key_of(handle) };
    reg().instances.get(&k).cloned()
}

fn dev_of(handle: u64) -> Option<Arc<Dev>> {
    let k = unsafe { key_of(handle) };
    reg().devices.get(&k).cloned()
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ---- exported layer functions ------------------------------------------------------------------------------

/// # Safety
/// Called by the Vulkan loader with a valid structure.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn kettle_vk_negotiate(v: *mut NegotiateInterface) -> vk::Result {
    if v.is_null() {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    }
    unsafe {
        (*v).version = (*v).version.min(2);
        (*v).gipa = Some(kettle_vk_gipa);
        (*v).gdpa = Some(kettle_vk_gdpa);
        (*v).gpdpa = std::ptr::null();
    }
    vk::Result::SUCCESS
}

macro_rules! ours {
    ($name:expr, $($n:literal => $f:expr),* $(,)?) => {
        match $name {
            $( $n => Some(unsafe { std::mem::transmute::<*const (), unsafe extern "system" fn()>($f as *const ()) }), )*
            _ => None,
        }
    };
}

fn our_device_fn(name: &str) -> vk::PFN_vkVoidFunction {
    ours!(name,
        "vkGetDeviceProcAddr" => kettle_vk_gdpa as unsafe extern "system" fn(_, _) -> _,
        "vkDestroyDevice" => destroy_device as unsafe extern "system" fn(_, _),
        "vkGetDeviceQueue" => get_device_queue as unsafe extern "system" fn(_, _, _, _),
        "vkGetDeviceQueue2" => get_device_queue2 as unsafe extern "system" fn(_, _, _),
        "vkCreateSwapchainKHR" => create_swapchain as unsafe extern "system" fn(_, _, _, _) -> _,
        "vkDestroySwapchainKHR" => destroy_swapchain as unsafe extern "system" fn(_, _, _),
        "vkQueuePresentKHR" => queue_present as unsafe extern "system" fn(_, _) -> _,
    )
}

fn our_instance_fn(name: &str) -> vk::PFN_vkVoidFunction {
    ours!(name,
        "vkGetInstanceProcAddr" => kettle_vk_gipa as unsafe extern "system" fn(_, _) -> _,
        "vkCreateInstance" => create_instance as unsafe extern "system" fn(_, _, _) -> _,
        "vkDestroyInstance" => destroy_instance as unsafe extern "system" fn(_, _),
        "vkCreateDevice" => create_device as unsafe extern "system" fn(_, _, _, _) -> _,
    )
}

/// # Safety
/// Called by the Vulkan loader.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn kettle_vk_gipa(instance: vk::Instance, name: *const c_char) -> vk::PFN_vkVoidFunction {
    if name.is_null() {
        return None;
    }
    let n = unsafe { CStr::from_ptr(name) }.to_str().unwrap_or("");
    if let Some(f) = our_instance_fn(n).or_else(|| our_device_fn(n)) {
        return Some(f);
    }
    let gipa = reg().gipas.get(&unsafe { key_of(instance.as_raw()) }).copied()?;
    unsafe { gipa(instance, name) }
}

/// # Safety
/// Called by the Vulkan loader.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn kettle_vk_gdpa(device: vk::Device, name: *const c_char) -> vk::PFN_vkVoidFunction {
    if name.is_null() {
        return None;
    }
    let n = unsafe { CStr::from_ptr(name) }.to_str().unwrap_or("");
    let gdpa = reg().gdpas.get(&unsafe { key_of(device.as_raw()) }).copied()?;
    // Only intercept what the device really supports (the swapchain functions need the extension).
    if let Some(f) = our_device_fn(n) {
        if n.contains("Swapchain") || n == "vkQueuePresentKHR" {
            let real = unsafe { gdpa(device, name) };
            real?;
        }
        return Some(f);
    }
    unsafe { gdpa(device, name) }
}

unsafe extern "system" fn create_instance(
    ci: *const vk::InstanceCreateInfo<'_>,
    alloc: *const vk::AllocationCallbacks<'_>,
    out: *mut vk::Instance,
) -> vk::Result {
    unsafe {
        let info = find_create_info((*ci).p_next, LOADER_INSTANCE_CREATE_INFO, LAYER_LINK_INFO);
        if info.is_null() || (*info).info.is_null() {
            return vk::Result::ERROR_INITIALIZATION_FAILED;
        }
        let link = (*info).info as *mut InstanceLink;
        let gipa = (*link).gipa;
        (*info).info = (*link).next.cast();
        let Some(create) = gipa(vk::Instance::null(), c"vkCreateInstance".as_ptr()) else {
            return vk::Result::ERROR_INITIALIZATION_FAILED;
        };
        let create: vk::PFN_vkCreateInstance = std::mem::transmute(create);
        let r = create(ci, alloc, out);
        if r != vk::Result::SUCCESS {
            return r;
        }
        let handle = *out;
        let key = key_of(handle.as_raw());
        reg().gipas.insert(key, gipa);
        crate::guard::run_always("vkCreateInstance", || {
            let ash = ash::Instance::load(&ash::StaticFn { get_instance_proc_addr: gipa }, handle);
            let inst = Arc::new(Inst { handle, ash, gipa });
            reg().instances.insert(key, inst);
        });
        vk::Result::SUCCESS
    }
}

unsafe extern "system" fn destroy_instance(instance: vk::Instance, alloc: *const vk::AllocationCallbacks<'_>) {
    let key = unsafe { key_of(instance.as_raw()) };
    let Some(inst) = inst_of(instance.as_raw()) else {
        // Not registered (our bookkeeping failed): the next layer's function still has to run.
        let gipa = reg().gipas.remove(&key);
        unsafe {
            if let Some(f) = gipa.and_then(|g| g(instance, c"vkDestroyInstance".as_ptr())) {
                let f: vk::PFN_vkDestroyInstance = std::mem::transmute(f);
                f(instance, alloc);
            }
        }
        return;
    };
    {
        let mut r = reg();
        r.instances.remove(&key);
        r.gipas.remove(&key);
    }
    unsafe {
        let alloc = alloc.as_ref();
        inst.ash.destroy_instance(alloc);
    }
}

unsafe extern "system" fn create_device(
    phys: vk::PhysicalDevice,
    ci: *const vk::DeviceCreateInfo<'_>,
    alloc: *const vk::AllocationCallbacks<'_>,
    out: *mut vk::Device,
) -> vk::Result {
    unsafe {
        let Some(inst) = inst_of(phys.as_raw()) else { return vk::Result::ERROR_INITIALIZATION_FAILED };
        let info = find_create_info((*ci).p_next, LOADER_DEVICE_CREATE_INFO, LAYER_LINK_INFO);
        if info.is_null() || (*info).info.is_null() {
            return vk::Result::ERROR_INITIALIZATION_FAILED;
        }
        let cb = find_create_info((*ci).p_next, LOADER_DEVICE_CREATE_INFO, LOADER_DATA_CALLBACK);
        let set_loader_data: Option<SetLoaderData> =
            if cb.is_null() || (*cb).info.is_null() { None } else { Some(std::mem::transmute((*cb).info)) };
        let link = (*info).info as *mut DeviceLink;
        let gipa = (*link).gipa;
        let gdpa = (*link).gdpa;
        (*info).info = (*link).next.cast();
        let Some(create) = gipa(inst.handle, c"vkCreateDevice".as_ptr()) else {
            return vk::Result::ERROR_INITIALIZATION_FAILED;
        };
        let create: vk::PFN_vkCreateDevice = std::mem::transmute(create);
        let r = create(phys, ci, alloc, out);
        if r != vk::Result::SUCCESS {
            return r;
        }
        let handle = *out;
        reg().gdpas.insert(key_of(handle.as_raw()), gdpa);
        crate::guard::run_always("vkCreateDevice", || register_device(inst, phys, handle, gdpa, set_loader_data));
        vk::Result::SUCCESS
    }
}

/// Everything we keep about a new device.
fn register_device(
    inst: Arc<Inst>,
    phys: vk::PhysicalDevice,
    handle: vk::Device,
    gdpa: vk::PFN_vkGetDeviceProcAddr,
    set_loader_data: Option<SetLoaderData>,
) {
    unsafe {
        let load = |n: &CStr| std::mem::transmute::<vk::PFN_vkVoidFunction, *const c_void>(gdpa(handle, n.as_ptr()));
        let ash = ash::Device::load_with(load, handle);
        let swapchain = ash::khr::swapchain::DeviceFn::load(load);
        let dev = Arc::new(Dev {
            handle,
            phys,
            ash,
            swapchain,
            gdpa,
            set_loader_data,
            families: inst.ash.get_physical_device_queue_family_properties(phys),
            mem: inst.ash.get_physical_device_memory_properties(phys),
            inst,
            queues: Mutex::new(HashMap::new()),
            swaps: Mutex::new(HashMap::new()),
            renderer: Mutex::new(None),
        });
        reg().devices.insert(key_of(handle.as_raw()), dev);
    }
}

unsafe extern "system" fn destroy_device(device: vk::Device, alloc: *const vk::AllocationCallbacks<'_>) {
    let key = unsafe { key_of(device.as_raw()) };
    let Some(dev) = dev_of(device.as_raw()) else {
        let gdpa = reg().gdpas.remove(&key);
        unsafe {
            if let Some(f) = gdpa.and_then(|g| g(device, c"vkDestroyDevice".as_ptr())) {
                let f: vk::PFN_vkDestroyDevice = std::mem::transmute(f);
                f(device, alloc);
            }
        }
        return;
    };
    {
        let mut r = reg();
        r.devices.remove(&key);
        r.gdpas.remove(&key);
    }
    // Our cleanup is guarded: the game's device is destroyed whatever happens to it.
    crate::guard::run_always("vkDestroyDevice", || unsafe {
        let _ = dev.ash.device_wait_idle();
        if let Some(mut r) = lock(&dev.renderer).take() {
            r.destroy(&dev);
        }
        for (id, s) in lock(&dev.swaps).drain() {
            s.destroy(&dev);
            state::release(id as usize);
        }
    });
    unsafe { dev.ash.destroy_device(alloc.as_ref()) };
}

unsafe extern "system" fn get_device_queue(device: vk::Device, family: u32, index: u32, out: *mut vk::Queue) {
    let Some(dev) = dev_of(device.as_raw()) else {
        // Not registered: straight to the next layer.
        let gdpa = reg().gdpas.get(&unsafe { key_of(device.as_raw()) }).copied();
        unsafe {
            if let Some(f) = gdpa.and_then(|g| g(device, c"vkGetDeviceQueue".as_ptr())) {
                let f: vk::PFN_vkGetDeviceQueue = std::mem::transmute(f);
                f(device, family, index, out);
            }
        }
        return;
    };
    unsafe {
        (dev.ash.fp_v1_0().get_device_queue)(device, family, index, out);
        crate::guard::run_always("vkGetDeviceQueue", || {
            if !(*out).is_null() {
                lock(&dev.queues).insert((*out).as_raw(), family);
            }
        });
    }
}

unsafe extern "system" fn get_device_queue2(
    device: vk::Device,
    info: *const vk::DeviceQueueInfo2<'_>,
    out: *mut vk::Queue,
) {
    let Some(dev) = dev_of(device.as_raw()) else {
        let gdpa = reg().gdpas.get(&unsafe { key_of(device.as_raw()) }).copied();
        unsafe {
            if let Some(f) = gdpa.and_then(|g| g(device, c"vkGetDeviceQueue2".as_ptr())) {
                let f: vk::PFN_vkGetDeviceQueue2 = std::mem::transmute(f);
                f(device, info, out);
            }
        }
        return;
    };
    unsafe {
        (dev.ash.fp_v1_1().get_device_queue2)(device, info, out);
        crate::guard::run_always("vkGetDeviceQueue2", || {
            if !(*out).is_null() {
                lock(&dev.queues).insert((*out).as_raw(), (*info).queue_family_index);
            }
        });
    }
}

/// Is `f` one a screenshot can read (four 8-bit channels, in either order)?
fn orig_readable_format(f: vk::Format) -> bool {
    matches!(
        f,
        vk::Format::B8G8R8A8_UNORM | vk::Format::B8G8R8A8_SRGB | vk::Format::R8G8B8A8_UNORM | vk::Format::R8G8B8A8_SRGB
    )
}

/// Formats the overlay can draw into, and whether the target is sRGB-encoded.
fn target_kind(f: vk::Format) -> Option<bool> {
    match f {
        vk::Format::B8G8R8A8_UNORM | vk::Format::R8G8B8A8_UNORM | vk::Format::A8B8G8R8_UNORM_PACK32 => Some(false),
        vk::Format::B8G8R8A8_SRGB | vk::Format::R8G8B8A8_SRGB | vk::Format::A8B8G8R8_SRGB_PACK32 => Some(true),
        _ => None,
    }
}

unsafe extern "system" fn create_swapchain(
    device: vk::Device,
    ci: *const vk::SwapchainCreateInfoKHR<'_>,
    alloc: *const vk::AllocationCallbacks<'_>,
    out: *mut vk::SwapchainKHR,
) -> vk::Result {
    let Some(dev) = dev_of(device.as_raw()) else { return vk::Result::ERROR_INITIALIZATION_FAILED };
    unsafe {
        let orig = &*ci;
        let drawable = target_kind(orig.image_format).is_some();
        let create = dev.swapchain.create_swapchain_khr;
        let (mut usable, mut readable) = (drawable, false);
        let mut r = vk::Result::ERROR_UNKNOWN;
        if drawable {
            // The usage the overlay (and a screenshot) needs on top of the game's; the first the driver takes.
            for (extra, can_read) in [
                (vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC, true),
                (vk::ImageUsageFlags::COLOR_ATTACHMENT, false),
            ] {
                let mut with_usage = *orig;
                with_usage.image_usage |= extra;
                r = create(device, &with_usage, alloc, out);
                if r == vk::Result::SUCCESS {
                    readable = can_read && orig_readable_format(orig.image_format);
                    break;
                }
            }
        }
        if r != vk::Result::SUCCESS {
            usable = false;
            r = create(device, ci, alloc, out);
        }
        if r == vk::Result::SUCCESS {
            // The image views are made without the game's mutable-format list: only the same format is used.
            crate::guard::run_always("vkCreateSwapchainKHR", || {
                lock(&dev.swaps).insert(
                    (*out).as_raw(),
                    Swap {
                        format: orig.image_format,
                        extent: orig.image_extent,
                        usable,
                        readable,
                        images: Vec::new(),
                        views: Vec::new(),
                        fbs: Vec::new(),
                        pass: vk::RenderPass::null(),
                    },
                );
            });
        }
        r
    }
}

unsafe extern "system" fn destroy_swapchain(
    device: vk::Device,
    swapchain: vk::SwapchainKHR,
    alloc: *const vk::AllocationCallbacks<'_>,
) {
    let Some(dev) = dev_of(device.as_raw()) else { return };
    unsafe {
        crate::guard::run_always("vkDestroySwapchainKHR", || {
            let removed = lock(&dev.swaps).remove(&swapchain.as_raw());
            if let Some(s) = removed {
                if let Some(r) = lock(&dev.renderer).as_mut() {
                    r.wait(&dev);
                }
                s.destroy(&dev);
            }
            state::release(swapchain.as_raw() as usize);
        });
        (dev.swapchain.destroy_swapchain_khr)(device, swapchain, alloc);
    }
}

impl Swap {
    unsafe fn destroy(&self, dev: &Dev) {
        unsafe {
            for f in &self.fbs {
                dev.ash.destroy_framebuffer(*f, None);
            }
            for v in &self.views {
                dev.ash.destroy_image_view(*v, None);
            }
        }
    }
}

unsafe extern "system" fn queue_present(queue: vk::Queue, info: *const vk::PresentInfoKHR<'_>) -> vk::Result {
    // Which device? A queue's dispatch key is its device's.
    let Some(dev) = dev_of(queue.as_raw()) else { return vk::Result::ERROR_DEVICE_LOST };
    let present = dev.swapchain.queue_present_khr;
    let result = crate::guard::run("vkQueuePresentKHR", || unsafe { draw(&dev, queue, &*info) });
    match result {
        Some(Some(sem)) => unsafe {
            let mut patched = *info;
            let sems = [sem];
            patched.wait_semaphore_count = 1;
            patched.p_wait_semaphores = sems.as_ptr();
            present(queue, &patched)
        },
        _ => unsafe { present(queue, info) },
    }
}

/// Draw the overlay into the images this present shows. Returns the semaphore the present must wait for
/// instead of the game's own, or `None` when nothing was drawn.
unsafe fn draw(dev: &Arc<Dev>, queue: vk::Queue, info: &vk::PresentInfoKHR<'_>) -> Option<vk::Semaphore> {
    state::sync_backends();
    let family = *lock(&dev.queues).get(&queue.as_raw())?;
    if !dev.families.get(family as usize)?.queue_flags.contains(vk::QueueFlags::GRAPHICS) {
        return None;
    }
    let n = info.swapchain_count as usize;
    if n == 0 {
        return None;
    }
    let swapchains = unsafe { std::slice::from_raw_parts(info.p_swapchains, n) };
    let indices = unsafe { std::slice::from_raw_parts(info.p_image_indices, n) };
    // One frame per present: the first swapchain that may draw gets it.
    let mut chosen = None;
    for (i, sc) in swapchains.iter().enumerate() {
        let id = sc.as_raw() as usize;
        let ok = lock(&dev.swaps).get(&sc.as_raw()).is_some_and(|s| s.usable);
        if !ok {
            continue;
        }
        match state::claim(id) {
            Claim::Other => {}
            c => {
                chosen = Some((i, c));
                break;
            }
        }
    }
    let (i, claim) = chosen?;
    let (w, h, readable, format) = {
        let swaps = lock(&dev.swaps);
        let s = swaps.get(&swapchains[i].as_raw())?;
        (s.extent.width, s.extent.height, s.readable, s.format)
    };
    if state::take_shot() && readable {
        unsafe { grab(dev, queue, family, swapchains[i], indices[i], (w, h), format) };
    }
    let frame = {
        let mut c = state::lock();
        c.tick();
        c.frame(w, h)
    };
    let mut frame = frame?;
    let mut guard = lock(&dev.renderer);
    if claim == Claim::New
        && let Some(r) = guard.as_mut()
    {
        unsafe { r.reset_textures(dev) };
    }
    if guard.is_none() {
        *guard = unsafe { Renderer::new(dev, family) };
    }
    let r = guard.as_mut()?;
    static FIRST: std::sync::Once = std::sync::Once::new();
    FIRST.call_once(|| crate::ipc::log("vk first frame"));
    if r.family != family {
        return None;
    }
    let waits = unsafe { std::slice::from_raw_parts(info.p_wait_semaphores, info.wait_semaphore_count as usize) };
    unsafe { r.render(dev, queue, swapchains[i], indices[i], waits, &mut frame) }
}

/// Copy the image this present shows (the game's frame, the overlay not yet drawn into it) to the CPU and save
/// it as a screenshot. Waits for the device to be idle first: the game's own rendering of this frame is already
/// submitted, and a screenshot is rare. Everything made here is destroyed again.
unsafe fn grab(
    dev: &Arc<Dev>,
    queue: vk::Queue,
    family: u32,
    swapchain: vk::SwapchainKHR,
    index: u32,
    (w, h): (u32, u32),
    format: vk::Format,
) {
    let bgra = matches!(format, vk::Format::B8G8R8A8_UNORM | vk::Format::B8G8R8A8_SRGB);
    if w == 0 || h == 0 || w > 16384 || h > 16384 {
        return;
    }
    unsafe {
        let a = &dev.ash;
        let get = dev.swapchain.get_swapchain_images_khr;
        let mut n = 0u32;
        if get(dev.handle, swapchain, &mut n, std::ptr::null_mut()) != vk::Result::SUCCESS || index >= n {
            return;
        }
        let mut imgs = vec![vk::Image::null(); n as usize];
        if get(dev.handle, swapchain, &mut n, imgs.as_mut_ptr()) != vk::Result::SUCCESS {
            return;
        }
        let image = imgs[index as usize];
        let size = u64::from(w) * u64::from(h) * 4;
        let Some(buf) = Buf::new(dev, size, vk::BufferUsageFlags::TRANSFER_DST) else { return };
        let pool = a.create_command_pool(
            &vk::CommandPoolCreateInfo::default()
                .queue_family_index(family)
                .flags(vk::CommandPoolCreateFlags::TRANSIENT),
            None,
        );
        let mut pixels = None;
        if let Ok(pool) = pool {
            let cmds = a.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            );
            let fence = a.create_fence(&vk::FenceCreateInfo::default(), None);
            if let (Ok(cmds), Ok(fence)) = (cmds, fence) {
                let cmd = cmds[0];
                let range = vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                };
                let barrier = |from: vk::ImageLayout, to: vk::ImageLayout, src, dst| {
                    vk::ImageMemoryBarrier::default()
                        .old_layout(from)
                        .new_layout(to)
                        .src_access_mask(src)
                        .dst_access_mask(dst)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .image(image)
                        .subresource_range(range)
                };
                let recorded = a
                    .begin_command_buffer(
                        cmd,
                        &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                    )
                    .is_ok();
                if recorded {
                    // The game left the image ready to present; wait for the device first so its work is done.
                    let _ = a.device_wait_idle();
                    a.cmd_pipeline_barrier(
                        cmd,
                        vk::PipelineStageFlags::ALL_COMMANDS,
                        vk::PipelineStageFlags::TRANSFER,
                        vk::DependencyFlags::empty(),
                        &[],
                        &[],
                        &[barrier(
                            vk::ImageLayout::PRESENT_SRC_KHR,
                            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                            vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE,
                            vk::AccessFlags::TRANSFER_READ,
                        )],
                    );
                    a.cmd_copy_image_to_buffer(
                        cmd,
                        image,
                        vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                        buf.buf,
                        &[vk::BufferImageCopy {
                            buffer_offset: 0,
                            buffer_row_length: 0,
                            buffer_image_height: 0,
                            image_subresource: vk::ImageSubresourceLayers {
                                aspect_mask: vk::ImageAspectFlags::COLOR,
                                mip_level: 0,
                                base_array_layer: 0,
                                layer_count: 1,
                            },
                            image_offset: vk::Offset3D::default(),
                            image_extent: vk::Extent3D { width: w, height: h, depth: 1 },
                        }],
                    );
                    a.cmd_pipeline_barrier(
                        cmd,
                        vk::PipelineStageFlags::TRANSFER,
                        vk::PipelineStageFlags::ALL_COMMANDS,
                        vk::DependencyFlags::empty(),
                        &[],
                        &[],
                        &[barrier(
                            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                            vk::ImageLayout::PRESENT_SRC_KHR,
                            vk::AccessFlags::TRANSFER_READ,
                            vk::AccessFlags::MEMORY_READ,
                        )],
                    );
                    let cmd_list = [cmd];
                    if a.end_command_buffer(cmd).is_ok()
                        && a.queue_submit(queue, &[vk::SubmitInfo::default().command_buffers(&cmd_list)], fence).is_ok()
                    {
                        if a.wait_for_fences(&[fence], true, FENCE_WAIT_NS).is_ok() {
                            pixels = Some(std::slice::from_raw_parts(buf.ptr, size as usize).to_vec());
                        } else {
                            // Still running: do not free what it uses.
                            let _ = a.device_wait_idle();
                        }
                    }
                }
                a.destroy_fence(fence, None);
            } else if let Ok(fence) = fence {
                a.destroy_fence(fence, None);
            }
            a.destroy_command_pool(pool, None);
        }
        buf.destroy(dev);
        if let Some(mut px) = pixels {
            for p in px.as_chunks_mut::<4>().0 {
                if bgra {
                    p.swap(0, 2);
                }
                p[3] = 255;
            }
            crate::shot::save(px, w, h);
        }
    }
}

// ---- the renderer ------------------------------------------------------------------------------------------

unsafe impl Send for Buf {}

struct Buf {
    buf: vk::Buffer,
    mem: vk::DeviceMemory,
    ptr: *mut u8,
    size: u64,
}

struct Tex {
    image: vk::Image,
    mem: vk::DeviceMemory,
    view: vk::ImageView,
    set: vk::DescriptorSet,
    #[allow(dead_code)]
    size: [usize; 2],
}

struct FrameRes {
    cmd: vk::CommandBuffer,
    fence: vk::Fence,
    sem: vk::Semaphore,
    vertices: Option<Buf>,
    indices: Option<Buf>,
    staging: Option<Buf>,
    used: bool,
    garbage_tex: Vec<Tex>,
}

struct Pipelines {
    pass: vk::RenderPass,
    gamma: vk::Pipeline,
    linear: vk::Pipeline,
}

struct Renderer {
    family: u32,
    pool: vk::CommandPool,
    layout_set: vk::DescriptorSetLayout,
    layout_pipe: vk::PipelineLayout,
    desc_pool: vk::DescriptorPool,
    sampler_linear: vk::Sampler,
    sampler_nearest: vk::Sampler,
    shader: vk::ShaderModule,
    pipes: HashMap<vk::Format, Pipelines>,
    frames: Vec<FrameRes>,
    cur: usize,
    textures: HashMap<egui::TextureId, Tex>,
}

const SPIRV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/overlay.spv"));

fn find_memory(dev: &Dev, bits: u32, want: vk::MemoryPropertyFlags) -> Option<u32> {
    (0..dev.mem.memory_type_count)
        .find(|i| bits & (1 << i) != 0 && dev.mem.memory_types[*i as usize].property_flags.contains(want))
}

impl Buf {
    unsafe fn new(dev: &Dev, size: u64, usage: vk::BufferUsageFlags) -> Option<Buf> {
        unsafe {
            let buf = dev.ash.create_buffer(&vk::BufferCreateInfo::default().size(size).usage(usage), None).ok()?;
            let req = dev.ash.get_buffer_memory_requirements(buf);
            let want = vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
            let Some(ty) = find_memory(dev, req.memory_type_bits, want) else {
                dev.ash.destroy_buffer(buf, None);
                return None;
            };
            let mem = dev
                .ash
                .allocate_memory(
                    &vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(ty),
                    None,
                )
                .ok();
            let Some(mem) = mem else {
                dev.ash.destroy_buffer(buf, None);
                return None;
            };
            let ptr = dev.ash.map_memory(mem, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty());
            match (ptr, dev.ash.bind_buffer_memory(buf, mem, 0)) {
                (Ok(ptr), Ok(())) => Some(Buf { buf, mem, ptr: ptr.cast(), size }),
                _ => {
                    dev.ash.free_memory(mem, None);
                    dev.ash.destroy_buffer(buf, None);
                    None
                }
            }
        }
    }

    unsafe fn destroy(self, dev: &Dev) {
        unsafe {
            dev.ash.unmap_memory(self.mem);
            dev.ash.destroy_buffer(self.buf, None);
            dev.ash.free_memory(self.mem, None);
        }
    }
}

/// Make sure `slot` holds a buffer of at least `need` bytes (the old contents are dropped).
unsafe fn ensure(dev: &Dev, slot: &mut Option<Buf>, need: u64, usage: vk::BufferUsageFlags) -> bool {
    if slot.as_ref().is_some_and(|b| b.size >= need) {
        return true;
    }
    if let Some(old) = slot.take() {
        unsafe { old.destroy(dev) };
    }
    *slot = unsafe { Buf::new(dev, (need * 2).max(64 * 1024), usage) };
    slot.is_some()
}

impl Tex {
    unsafe fn destroy(self, dev: &Dev, pool: vk::DescriptorPool) {
        unsafe {
            let _ = dev.ash.free_descriptor_sets(pool, &[self.set]);
            dev.ash.destroy_image_view(self.view, None);
            dev.ash.destroy_image(self.image, None);
            dev.ash.free_memory(self.mem, None);
        }
    }
}

impl Renderer {
    unsafe fn new(dev: &Dev, family: u32) -> Option<Renderer> {
        unsafe {
            let a = &dev.ash;
            let pool = a
                .create_command_pool(
                    &vk::CommandPoolCreateInfo::default()
                        .queue_family_index(family)
                        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                    None,
                )
                .ok()?;
            let bindings = [
                vk::DescriptorSetLayoutBinding::default()
                    .binding(0)
                    .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::FRAGMENT),
                vk::DescriptorSetLayoutBinding::default()
                    .binding(1)
                    .descriptor_type(vk::DescriptorType::SAMPLER)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            ];
            let layout_set = a
                .create_descriptor_set_layout(&vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings), None)
                .ok()?;
            let sets = [layout_set];
            let layout_pipe =
                a.create_pipeline_layout(&vk::PipelineLayoutCreateInfo::default().set_layouts(&sets), None).ok()?;
            let sizes = [
                vk::DescriptorPoolSize::default().ty(vk::DescriptorType::SAMPLED_IMAGE).descriptor_count(128),
                vk::DescriptorPoolSize::default().ty(vk::DescriptorType::SAMPLER).descriptor_count(128),
            ];
            let desc_pool = a
                .create_descriptor_pool(
                    &vk::DescriptorPoolCreateInfo::default()
                        .flags(vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET)
                        .max_sets(128)
                        .pool_sizes(&sizes),
                    None,
                )
                .ok()?;
            let mk = |f: vk::Filter| {
                a.create_sampler(
                    &vk::SamplerCreateInfo::default()
                        .mag_filter(f)
                        .min_filter(f)
                        .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                        .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                        .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE),
                    None,
                )
            };
            let sampler_linear = mk(vk::Filter::LINEAR).ok()?;
            let sampler_nearest = mk(vk::Filter::NEAREST).ok()?;
            let words: Vec<u32> = (0..SPIRV.len() / 4)
                .map(|i| u32::from_le_bytes([SPIRV[i * 4], SPIRV[i * 4 + 1], SPIRV[i * 4 + 2], SPIRV[i * 4 + 3]]))
                .collect();
            let shader = a.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&words), None).ok()?;
            let cmds = a
                .allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(pool)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(FRAMES as u32),
                )
                .ok()?;
            let mut frames = Vec::new();
            for cmd in cmds {
                // Command buffers the layer makes itself need the loader's dispatch pointer set.
                if let Some(set) = dev.set_loader_data {
                    let _ = set(dev.handle, cmd.as_raw() as *mut c_void);
                }
                let fence =
                    a.create_fence(&vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED), None).ok()?;
                let sem = a.create_semaphore(&vk::SemaphoreCreateInfo::default(), None).ok()?;
                frames.push(FrameRes {
                    cmd,
                    fence,
                    sem,
                    vertices: None,
                    indices: None,
                    staging: None,
                    used: false,
                    garbage_tex: vec![],
                });
            }
            Some(Renderer {
                family,
                pool,
                layout_set,
                layout_pipe,
                desc_pool,
                sampler_linear,
                sampler_nearest,
                shader,
                pipes: HashMap::new(),
                frames,
                cur: 0,
                textures: HashMap::new(),
            })
        }
    }

    unsafe fn wait(&mut self, dev: &Dev) {
        let fences: Vec<vk::Fence> = self.frames.iter().filter(|f| f.used).map(|f| f.fence).collect();
        if !fences.is_empty() {
            unsafe {
                let _ = dev.ash.wait_for_fences(&fences, true, FENCE_WAIT_NS);
            }
        }
    }

    unsafe fn reset_textures(&mut self, dev: &Dev) {
        unsafe {
            self.wait(dev);
            for (_, t) in self.textures.drain() {
                t.destroy(dev, self.desc_pool);
            }
        }
    }

    unsafe fn destroy(&mut self, dev: &Dev) {
        unsafe {
            let a = &dev.ash;
            self.wait(dev);
            for (_, t) in self.textures.drain() {
                t.destroy(dev, self.desc_pool);
            }
            for mut f in self.frames.drain(..) {
                for t in f.garbage_tex.drain(..) {
                    t.destroy(dev, self.desc_pool);
                }
                for b in [f.vertices.take(), f.indices.take(), f.staging.take()].into_iter().flatten() {
                    b.destroy(dev);
                }
                a.destroy_fence(f.fence, None);
                a.destroy_semaphore(f.sem, None);
            }
            for (_, p) in self.pipes.drain() {
                a.destroy_pipeline(p.gamma, None);
                a.destroy_pipeline(p.linear, None);
                a.destroy_render_pass(p.pass, None);
            }
            a.destroy_shader_module(self.shader, None);
            a.destroy_sampler(self.sampler_linear, None);
            a.destroy_sampler(self.sampler_nearest, None);
            a.destroy_descriptor_pool(self.desc_pool, None);
            a.destroy_pipeline_layout(self.layout_pipe, None);
            a.destroy_descriptor_set_layout(self.layout_set, None);
            a.destroy_command_pool(self.pool, None);
        }
    }

    /// The render pass and pipelines for a target format (made once).
    unsafe fn pipelines(&mut self, dev: &Dev, format: vk::Format) -> Option<(vk::RenderPass, vk::Pipeline)> {
        let srgb = target_kind(format)?;
        if let Some(p) = self.pipes.get(&format) {
            return Some((p.pass, if srgb { p.linear } else { p.gamma }));
        }
        unsafe {
            let a = &dev.ash;
            let att = [vk::AttachmentDescription::default()
                .format(format)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::LOAD)
                .store_op(vk::AttachmentStoreOp::STORE)
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                .initial_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                .final_layout(vk::ImageLayout::PRESENT_SRC_KHR)];
            let refs =
                [vk::AttachmentReference::default().attachment(0).layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)];
            let sub = [vk::SubpassDescription::default()
                .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
                .color_attachments(&refs)];
            let dep = [vk::SubpassDependency::default()
                .src_subpass(vk::SUBPASS_EXTERNAL)
                .dst_subpass(0)
                .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
                .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
                .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE)];
            let pass = a
                .create_render_pass(
                    &vk::RenderPassCreateInfo::default().attachments(&att).subpasses(&sub).dependencies(&dep),
                    None,
                )
                .ok()?;
            let mk = |entry: &CStr| -> Option<vk::Pipeline> {
                let stages = [
                    vk::PipelineShaderStageCreateInfo::default()
                        .stage(vk::ShaderStageFlags::VERTEX)
                        .module(self.shader)
                        .name(c"vs_main"),
                    vk::PipelineShaderStageCreateInfo::default()
                        .stage(vk::ShaderStageFlags::FRAGMENT)
                        .module(self.shader)
                        .name(entry),
                ];
                let binds = [vk::VertexInputBindingDescription::default()
                    .binding(0)
                    .stride(20)
                    .input_rate(vk::VertexInputRate::VERTEX)];
                let attrs = [
                    vk::VertexInputAttributeDescription::default()
                        .location(0)
                        .binding(0)
                        .format(vk::Format::R32G32_SFLOAT)
                        .offset(0),
                    vk::VertexInputAttributeDescription::default()
                        .location(1)
                        .binding(0)
                        .format(vk::Format::R32G32_SFLOAT)
                        .offset(8),
                    vk::VertexInputAttributeDescription::default()
                        .location(2)
                        .binding(0)
                        .format(vk::Format::R8G8B8A8_UNORM)
                        .offset(16),
                ];
                let vi = vk::PipelineVertexInputStateCreateInfo::default()
                    .vertex_binding_descriptions(&binds)
                    .vertex_attribute_descriptions(&attrs);
                let ia =
                    vk::PipelineInputAssemblyStateCreateInfo::default().topology(vk::PrimitiveTopology::TRIANGLE_LIST);
                let vp = vk::PipelineViewportStateCreateInfo::default().viewport_count(1).scissor_count(1);
                let rs = vk::PipelineRasterizationStateCreateInfo::default()
                    .polygon_mode(vk::PolygonMode::FILL)
                    .cull_mode(vk::CullModeFlags::NONE)
                    .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
                    .line_width(1.0);
                let ms = vk::PipelineMultisampleStateCreateInfo::default()
                    .rasterization_samples(vk::SampleCountFlags::TYPE_1);
                let blend_att = [vk::PipelineColorBlendAttachmentState::default()
                    .blend_enable(true)
                    .src_color_blend_factor(vk::BlendFactor::ONE)
                    .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
                    .color_blend_op(vk::BlendOp::ADD)
                    .src_alpha_blend_factor(vk::BlendFactor::ONE)
                    .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
                    .alpha_blend_op(vk::BlendOp::ADD)
                    .color_write_mask(vk::ColorComponentFlags::RGBA)];
                let blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_att);
                let dyn_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
                let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dyn_states);
                let ci = [vk::GraphicsPipelineCreateInfo::default()
                    .stages(&stages)
                    .vertex_input_state(&vi)
                    .input_assembly_state(&ia)
                    .viewport_state(&vp)
                    .rasterization_state(&rs)
                    .multisample_state(&ms)
                    .color_blend_state(&blend)
                    .dynamic_state(&dynamic)
                    .layout(self.layout_pipe)
                    .render_pass(pass)
                    .subpass(0)];
                a.create_graphics_pipelines(vk::PipelineCache::null(), &ci, None)
                    .ok()
                    .and_then(|v| v.into_iter().next())
            };
            let (Some(gamma), Some(linear)) = (mk(c"fs_gamma"), mk(c"fs_linear")) else {
                a.destroy_render_pass(pass, None);
                return None;
            };
            self.pipes.insert(format, Pipelines { pass, gamma, linear });
            Some((pass, if srgb { linear } else { gamma }))
        }
    }

    /// Record and submit one frame. Returns the semaphore the present waits for.
    unsafe fn render(
        &mut self,
        dev: &Dev,
        queue: vk::Queue,
        swapchain: vk::SwapchainKHR,
        index: u32,
        waits: &[vk::Semaphore],
        frame: &mut state::Frame,
    ) -> Option<vk::Semaphore> {
        unsafe {
            let a = &dev.ash;
            let slot = self.cur;
            self.cur = (self.cur + 1) % FRAMES;
            // The slot is free once its last submission finished.
            if self.frames[slot].used {
                a.wait_for_fences(&[self.frames[slot].fence], true, FENCE_WAIT_NS).ok()?;
            }
            let garbage: Vec<Tex> = std::mem::take(&mut self.frames[slot].garbage_tex);
            for t in garbage {
                t.destroy(dev, self.desc_pool);
            }
            let (format, extent) = {
                let swaps = lock(&dev.swaps);
                let s = swaps.get(&swapchain.as_raw())?;
                (s.format, s.extent)
            };
            let (pass, pipeline) = self.pipelines(dev, format)?;
            let fb = self.framebuffer(dev, swapchain, index, pass)?;

            let cmd = self.frames[slot].cmd;
            a.reset_command_buffer(cmd, vk::CommandBufferResetFlags::empty()).ok()?;
            a.begin_command_buffer(
                cmd,
                &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )
            .ok()?;

            self.upload_textures(dev, slot, cmd, frame)?;

            // Geometry: every mesh into one vertex and one index buffer, positions already in clip space.
            let (mut verts, mut idx) = (Vec::<u8>::new(), Vec::<u32>::new());
            struct Draw {
                first: u32,
                count: u32,
                base: i32,
                clip: [i32; 4],
                tex: egui::TextureId,
            }
            let mut draws = Vec::new();
            let (fw, fh) = (extent.width as f32, extent.height as f32);
            let ppp = frame.pixels_per_point;
            for p in &frame.primitives {
                let egui::epaint::Primitive::Mesh(m) = &p.primitive else { continue };
                if m.indices.is_empty() {
                    continue;
                }
                let base = (verts.len() / 20) as i32;
                for v in &m.vertices {
                    let x = v.pos.x * ppp / fw * 2.0 - 1.0;
                    let y = v.pos.y * ppp / fh * 2.0 - 1.0;
                    for f in [x, y, v.uv.x, v.uv.y] {
                        verts.extend_from_slice(&f.to_le_bytes());
                    }
                    verts.extend_from_slice(&v.color.to_array());
                }
                let first = idx.len() as u32;
                idx.extend_from_slice(&m.indices);
                let c = p.clip_rect;
                let x0 = ((c.min.x * ppp).floor() as i32).clamp(0, extent.width as i32);
                let y0 = ((c.min.y * ppp).floor() as i32).clamp(0, extent.height as i32);
                let x1 = ((c.max.x * ppp).ceil() as i32).clamp(0, extent.width as i32);
                let y1 = ((c.max.y * ppp).ceil() as i32).clamp(0, extent.height as i32);
                if x1 <= x0 || y1 <= y0 {
                    continue;
                }
                draws.push(Draw {
                    first,
                    count: m.indices.len() as u32,
                    base,
                    clip: [x0, y0, x1 - x0, y1 - y0],
                    tex: m.texture_id,
                });
            }
            if !draws.is_empty() {
                let f = &mut self.frames[slot];
                if !ensure(dev, &mut f.vertices, verts.len() as u64, vk::BufferUsageFlags::VERTEX_BUFFER)
                    || !ensure(dev, &mut f.indices, (idx.len() * 4) as u64, vk::BufferUsageFlags::INDEX_BUFFER)
                {
                    return None;
                }
                std::ptr::copy_nonoverlapping(verts.as_ptr(), f.vertices.as_ref()?.ptr, verts.len());
                std::ptr::copy_nonoverlapping(idx.as_ptr().cast::<u8>(), f.indices.as_ref()?.ptr, idx.len() * 4);
                a.cmd_begin_render_pass(
                    cmd,
                    &vk::RenderPassBeginInfo::default()
                        .render_pass(pass)
                        .framebuffer(fb)
                        .render_area(vk::Rect2D { offset: vk::Offset2D::default(), extent }),
                    vk::SubpassContents::INLINE,
                );
                a.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, pipeline);
                a.cmd_set_viewport(
                    cmd,
                    0,
                    &[vk::Viewport { x: 0.0, y: 0.0, width: fw, height: fh, min_depth: 0.0, max_depth: 1.0 }],
                );
                a.cmd_bind_vertex_buffers(cmd, 0, &[f.vertices.as_ref()?.buf], &[0]);
                a.cmd_bind_index_buffer(cmd, f.indices.as_ref()?.buf, 0, vk::IndexType::UINT32);
                for d in &draws {
                    let Some(t) = self.textures.get(&d.tex) else { continue };
                    a.cmd_bind_descriptor_sets(
                        cmd,
                        vk::PipelineBindPoint::GRAPHICS,
                        self.layout_pipe,
                        0,
                        &[t.set],
                        &[],
                    );
                    a.cmd_set_scissor(
                        cmd,
                        0,
                        &[vk::Rect2D {
                            offset: vk::Offset2D { x: d.clip[0], y: d.clip[1] },
                            extent: vk::Extent2D { width: d.clip[2] as u32, height: d.clip[3] as u32 },
                        }],
                    );
                    a.cmd_draw_indexed(cmd, d.count, 1, d.first, d.base, 0);
                }
                a.cmd_end_render_pass(cmd);
            }
            a.end_command_buffer(cmd).ok()?;

            // Free the textures egui is done with after this frame.
            let freed: Vec<egui::TextureId> = std::mem::take(&mut frame.textures.free).into_iter().collect();
            for id in freed {
                if let Some(t) = self.textures.remove(&id) {
                    self.frames[slot].garbage_tex.push(t);
                }
            }

            let f = &mut self.frames[slot];
            a.reset_fences(&[f.fence]).ok()?;
            let stages = vec![vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT; waits.len()];
            let cmds = [cmd];
            let sems = [f.sem];
            let submit = vk::SubmitInfo::default()
                .wait_semaphores(waits)
                .wait_dst_stage_mask(&stages)
                .command_buffers(&cmds)
                .signal_semaphores(&sems);
            if a.queue_submit(queue, &[submit], f.fence).is_err() {
                // Leave the fence signaled so the slot is reusable.
                let _ = a.queue_submit(queue, &[], f.fence);
                return None;
            }
            f.used = true;
            Some(f.sem)
        }
    }

    unsafe fn framebuffer(
        &mut self,
        dev: &Dev,
        swapchain: vk::SwapchainKHR,
        index: u32,
        pass: vk::RenderPass,
    ) -> Option<vk::Framebuffer> {
        unsafe {
            let mut swaps = lock(&dev.swaps);
            let s = swaps.get_mut(&swapchain.as_raw())?;
            if s.images.is_empty() || s.pass != pass {
                for f in s.fbs.drain(..) {
                    dev.ash.destroy_framebuffer(f, None);
                }
                for v in s.views.drain(..) {
                    dev.ash.destroy_image_view(v, None);
                }
                let mut n = 0u32;
                let get = dev.swapchain.get_swapchain_images_khr;
                if get(dev.handle, swapchain, &mut n, std::ptr::null_mut()) != vk::Result::SUCCESS {
                    return None;
                }
                let mut imgs = vec![vk::Image::null(); n as usize];
                if get(dev.handle, swapchain, &mut n, imgs.as_mut_ptr()) != vk::Result::SUCCESS {
                    return None;
                }
                imgs.truncate(n as usize);
                s.images = imgs;
                for img in &s.images {
                    let view = dev
                        .ash
                        .create_image_view(
                            &vk::ImageViewCreateInfo::default()
                                .image(*img)
                                .view_type(vk::ImageViewType::TYPE_2D)
                                .format(s.format)
                                .subresource_range(vk::ImageSubresourceRange {
                                    aspect_mask: vk::ImageAspectFlags::COLOR,
                                    base_mip_level: 0,
                                    level_count: 1,
                                    base_array_layer: 0,
                                    layer_count: 1,
                                }),
                            None,
                        )
                        .ok()?;
                    s.views.push(view);
                    let views = [view];
                    let fb = dev
                        .ash
                        .create_framebuffer(
                            &vk::FramebufferCreateInfo::default()
                                .render_pass(pass)
                                .attachments(&views)
                                .width(s.extent.width)
                                .height(s.extent.height)
                                .layers(1),
                            None,
                        )
                        .ok()?;
                    s.fbs.push(fb);
                }
                s.pass = pass;
            }
            s.fbs.get(index as usize).copied()
        }
    }

    /// Apply egui's texture changes: create or update images, with the pixels going through a staging buffer.
    unsafe fn upload_textures(
        &mut self,
        dev: &Dev,
        slot: usize,
        cmd: vk::CommandBuffer,
        frame: &mut state::Frame,
    ) -> Option<()> {
        unsafe {
            let a = &dev.ash;
            let mut items: Vec<(egui::TextureId, egui::epaint::ImageDelta)> = Vec::new();
            for (id, deltas) in std::mem::take(&mut frame.textures.set) {
                for d in deltas.into_iter() {
                    items.push((id, d));
                }
            }
            if items.is_empty() {
                return Some(());
            }
            let rgba = |d: &egui::epaint::ImageDelta| -> Vec<u8> {
                let egui::ImageData::Color(img) = &d.image;
                img.pixels.iter().flat_map(|p| p.to_array()).collect()
            };
            let total: usize = items.iter().map(|(_, d)| d.image.width() * d.image.height() * 4).sum();
            if !ensure(dev, &mut self.frames[slot].staging, total as u64, vk::BufferUsageFlags::TRANSFER_SRC) {
                return None;
            }
            let stage_ptr = self.frames[slot].staging.as_ref()?.ptr;
            let stage_buf = self.frames[slot].staging.as_ref()?.buf;
            let mut offset = 0usize;
            let sub = vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            };
            for (id, d) in items {
                let [w, h] = [d.image.width(), d.image.height()];
                let bytes = rgba(&d);
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), stage_ptr.add(offset), bytes.len());
                let (pos, old_layout) = match d.pos {
                    Some(p) if self.textures.contains_key(&id) => (p, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL),
                    _ => {
                        // A whole new image replaces any old one.
                        if let Some(old) = self.textures.remove(&id) {
                            self.frames[slot].garbage_tex.push(old);
                        }
                        let t = self.new_texture(dev, w, h, d.options.magnification)?;
                        self.textures.insert(id, t);
                        ([0, 0], vk::ImageLayout::UNDEFINED)
                    }
                };
                let t = self.textures.get(&id)?;
                let barrier =
                    |old: vk::ImageLayout, new: vk::ImageLayout, src: vk::AccessFlags, dst: vk::AccessFlags| {
                        vk::ImageMemoryBarrier::default()
                            .old_layout(old)
                            .new_layout(new)
                            .src_access_mask(src)
                            .dst_access_mask(dst)
                            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                            .image(t.image)
                            .subresource_range(sub)
                    };
                a.cmd_pipeline_barrier(
                    cmd,
                    vk::PipelineStageFlags::FRAGMENT_SHADER | vk::PipelineStageFlags::TOP_OF_PIPE,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[barrier(
                        old_layout,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        vk::AccessFlags::SHADER_READ,
                        vk::AccessFlags::TRANSFER_WRITE,
                    )],
                );
                let region = vk::BufferImageCopy::default()
                    .buffer_offset(offset as u64)
                    .image_subresource(vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        mip_level: 0,
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .image_offset(vk::Offset3D { x: pos[0] as i32, y: pos[1] as i32, z: 0 })
                    .image_extent(vk::Extent3D { width: w as u32, height: h as u32, depth: 1 });
                a.cmd_copy_buffer_to_image(cmd, stage_buf, t.image, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &[region]);
                a.cmd_pipeline_barrier(
                    cmd,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::FRAGMENT_SHADER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[barrier(
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                        vk::AccessFlags::TRANSFER_WRITE,
                        vk::AccessFlags::SHADER_READ,
                    )],
                );
                offset += bytes.len();
            }
            Some(())
        }
    }

    unsafe fn new_texture(&mut self, dev: &Dev, w: usize, h: usize, mag: egui::TextureFilter) -> Option<Tex> {
        unsafe {
            let a = &dev.ash;
            let image = a
                .create_image(
                    &vk::ImageCreateInfo::default()
                        .image_type(vk::ImageType::TYPE_2D)
                        .format(vk::Format::R8G8B8A8_UNORM)
                        .extent(vk::Extent3D { width: w as u32, height: h as u32, depth: 1 })
                        .mip_levels(1)
                        .array_layers(1)
                        .samples(vk::SampleCountFlags::TYPE_1)
                        .tiling(vk::ImageTiling::OPTIMAL)
                        .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST)
                        .initial_layout(vk::ImageLayout::UNDEFINED),
                    None,
                )
                .ok()?;
            let req = a.get_image_memory_requirements(image);
            let ty = find_memory(dev, req.memory_type_bits, vk::MemoryPropertyFlags::DEVICE_LOCAL)
                .or_else(|| find_memory(dev, req.memory_type_bits, vk::MemoryPropertyFlags::empty()))?;
            let mem = a
                .allocate_memory(
                    &vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(ty),
                    None,
                )
                .ok()?;
            a.bind_image_memory(image, mem, 0).ok()?;
            let view = a
                .create_image_view(
                    &vk::ImageViewCreateInfo::default()
                        .image(image)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(vk::Format::R8G8B8A8_UNORM)
                        .subresource_range(vk::ImageSubresourceRange {
                            aspect_mask: vk::ImageAspectFlags::COLOR,
                            base_mip_level: 0,
                            level_count: 1,
                            base_array_layer: 0,
                            layer_count: 1,
                        }),
                    None,
                )
                .ok()?;
            let layouts = [self.layout_set];
            let set = a
                .allocate_descriptor_sets(
                    &vk::DescriptorSetAllocateInfo::default().descriptor_pool(self.desc_pool).set_layouts(&layouts),
                )
                .ok()?[0];
            let sampler = if mag == egui::TextureFilter::Nearest { self.sampler_nearest } else { self.sampler_linear };
            let img_info = [vk::DescriptorImageInfo::default()
                .image_view(view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            let samp_info = [vk::DescriptorImageInfo::default().sampler(sampler)];
            a.update_descriptor_sets(
                &[
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(0)
                        .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                        .image_info(&img_info),
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(1)
                        .descriptor_type(vk::DescriptorType::SAMPLER)
                        .image_info(&samp_info),
                ],
                &[],
            );
            Some(Tex { image, mem, view, set, size: [w, h] })
        }
    }
}
