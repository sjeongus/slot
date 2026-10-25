/* The few libretro definitions rc_libretro.c needs, so slot need not vendor libretro.h. Values
 * and layouts match libretro-common's libretro.h. */
#ifndef SLOT_LIBRETRO_SHIM_H
#define SLOT_LIBRETRO_SHIM_H

#include <stddef.h>
#include <stdint.h>

#define RETRO_MEMORY_SAVE_RAM 0
#define RETRO_MEMORY_RTC 1
#define RETRO_MEMORY_SYSTEM_RAM 2
#define RETRO_MEMORY_VIDEO_RAM 3

#define RETRO_MEMDESC_CONST (1 << 0)
#define RETRO_MEMDESC_BIGENDIAN (1 << 1)
#define RETRO_MEMDESC_SYSTEM_RAM (1 << 2)
#define RETRO_MEMDESC_SAVE_RAM (1 << 3)
#define RETRO_MEMDESC_VIDEO_RAM (1 << 4)

struct retro_memory_descriptor {
  uint64_t flags;
  void* ptr;
  size_t offset;
  size_t start;
  size_t select;
  size_t disconnect;
  size_t len;
  const char* addrspace;
};

struct retro_memory_map {
  const struct retro_memory_descriptor* descriptors;
  unsigned num_descriptors;
};

#endif
