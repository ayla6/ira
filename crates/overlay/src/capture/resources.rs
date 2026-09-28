//! GPU resource construction for (re)builds: staging ring + intermediate.
//! Only caller is `build_resources` in the lifecycle module.

use ash::vk;

use super::{IntermediateImage, StagingBuffer};
use crate::types::DeviceFns;

const NUM_STAGING: usize = 4;

pub(super) fn create_staging_buffers(
    fns: DeviceFns,
    device: vk::Device,
    physical_device: vk::PhysicalDevice,
    buf_size: u64,
) -> Vec<StagingBuffer> {
    let mut buffers = Vec::with_capacity(NUM_STAGING);
    for _ in 0..NUM_STAGING {
        let Some(staging) =
            (unsafe { create_one_staging_buffer(fns, device, physical_device, buf_size) })
        else {
            break;
        };
        buffers.push(staging);
    }
    buffers
}

unsafe fn create_one_staging_buffer(
    fns: DeviceFns,
    device: vk::Device,
    physical_device: vk::PhysicalDevice,
    size: u64,
) -> Option<StagingBuffer> {
    let info = vk::BufferCreateInfo::default()
        .size(size)
        .usage(vk::BufferUsageFlags::TRANSFER_DST)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let mut buffer = vk::Buffer::null();
    if (fns.create_buffer)(device, &info, std::ptr::null(), &mut buffer) != vk::Result::SUCCESS {
        return None;
    }

    let mut reqs = vk::MemoryRequirements::default();
    (fns.get_buffer_memory_requirements)(device, buffer, &mut reqs);

    let mut mem_props = vk::PhysicalDeviceMemoryProperties::default();
    (fns.get_mem_props)(physical_device, &mut mem_props);

    let find_mem =
        |require: vk::MemoryPropertyFlags, avoid: vk::MemoryPropertyFlags| -> Option<u32> {
            for i in 0..mem_props.memory_type_count as usize {
                if (reqs.memory_type_bits & (1 << i)) != 0 {
                    let flags = mem_props.memory_types[i].property_flags;
                    if flags.contains(require) && !flags.intersects(avoid) {
                        return Some(i as u32);
                    }
                }
            }
            None
        };

    let mem_type = find_mem(
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
    )
    .or_else(|| {
        find_mem(
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            vk::MemoryPropertyFlags::empty(),
        )
    });
    let mem_type = mem_type?;

    let alloc_info = vk::MemoryAllocateInfo::default()
        .allocation_size(reqs.size)
        .memory_type_index(mem_type);
    let mut memory = vk::DeviceMemory::null();
    if (fns.allocate_memory)(device, &alloc_info, std::ptr::null(), &mut memory)
        != vk::Result::SUCCESS
    {
        (fns.destroy_buffer)(device, buffer, std::ptr::null());
        return None;
    }
    if (fns.bind_buffer_memory)(device, buffer, memory, 0) != vk::Result::SUCCESS {
        (fns.free_memory)(device, memory, std::ptr::null());
        (fns.destroy_buffer)(device, buffer, std::ptr::null());
        return None;
    }

    let mut ptr = std::ptr::null_mut();
    if (fns.map_memory)(
        device,
        memory,
        0,
        reqs.size,
        vk::MemoryMapFlags::empty(),
        &mut ptr,
    ) != vk::Result::SUCCESS
    {
        (fns.free_memory)(device, memory, std::ptr::null());
        (fns.destroy_buffer)(device, buffer, std::ptr::null());
        return None;
    }

    Some(StagingBuffer {
        buffer,
        memory,
        ptr: ptr as *mut u8,
        size: reqs.size,
    })
}

pub(super) unsafe fn create_intermediate_image(
    fns: DeviceFns,
    device: vk::Device,
    physical_device: vk::PhysicalDevice,
    extent: vk::Extent2D,
) -> Option<IntermediateImage> {
    let info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(vk::Format::R8G8B8A8_UNORM)
        .extent(vk::Extent3D {
            width: extent.width,
            height: extent.height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::TRANSFER_SRC)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED);

    let mut image = vk::Image::null();
    if (fns.create_image)(device, &info, std::ptr::null(), &mut image) != vk::Result::SUCCESS {
        return None;
    }

    let mut reqs = vk::MemoryRequirements::default();
    (fns.get_image_memory_requirements)(device, image, &mut reqs);

    let mut mem_props = vk::PhysicalDeviceMemoryProperties::default();
    (fns.get_mem_props)(physical_device, &mut mem_props);

    let mem_type = (0..mem_props.memory_type_count as usize)
        .find(|&i| {
            (reqs.memory_type_bits & (1 << i)) != 0
                && mem_props.memory_types[i]
                    .property_flags
                    .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
        })
        .map(|i| i as u32)
        .or_else(|| {
            (0..mem_props.memory_type_count as usize)
                .find(|&i| (reqs.memory_type_bits & (1 << i)) != 0)
                .map(|i| i as u32)
        });
    let mem_type = mem_type?;

    let alloc_info = vk::MemoryAllocateInfo::default()
        .allocation_size(reqs.size)
        .memory_type_index(mem_type);
    let mut memory = vk::DeviceMemory::null();
    if (fns.allocate_memory)(device, &alloc_info, std::ptr::null(), &mut memory)
        != vk::Result::SUCCESS
    {
        (fns.destroy_image)(device, image, std::ptr::null());
        return None;
    }
    if (fns.bind_image_memory)(device, image, memory, 0) != vk::Result::SUCCESS {
        (fns.free_memory)(device, memory, std::ptr::null());
        (fns.destroy_image)(device, image, std::ptr::null());
        return None;
    }

    Some(IntermediateImage { image, memory })
}
