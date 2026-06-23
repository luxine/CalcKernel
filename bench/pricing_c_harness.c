#define _POSIX_C_SOURCE 200809L

#include "pricing.h"

#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

#if defined(_WIN32)
#include <windows.h>
#else
#include <time.h>
#endif

static uint64_t now_ns(void) {
#if defined(_WIN32)
  LARGE_INTEGER frequency;
  LARGE_INTEGER counter;

  QueryPerformanceFrequency(&frequency);
  QueryPerformanceCounter(&counter);

  return (uint64_t)((counter.QuadPart * 1000000000ull) / frequency.QuadPart);
#else
  struct timespec ts;
  clock_gettime(CLOCK_MONOTONIC, &ts);
  return ((uint64_t)ts.tv_sec * 1000000000ull) + (uint64_t)ts.tv_nsec;
#endif
}

static void fill_items(Item* items, int32_t len) {
  for (int32_t i = 0; i < len; i += 1) {
    items[i].price = 1000 + (i % 997);
    items[i].qty = 1 + (i % 9);
    items[i].discount = i % 113;
    items[i].tax_rate_ppm = 50000 + (i % 17) * 2500;
  }
}

static int64_t checksum(const int64_t* values, int32_t len) {
  int64_t total = 0;

  for (int32_t i = 0; i < len; i += 1) {
    total += values[i];
  }

  return total;
}

int main(void) {
  const int32_t sizes[] = {100, 1000, 10000, 100000};
  const size_t size_count = sizeof(sizes) / sizeof(sizes[0]);

  for (size_t size_index = 0; size_index < size_count; size_index += 1) {
    const int32_t len = sizes[size_index];
    Item* items = (Item*)malloc(sizeof(Item) * (size_t)len);
    int64_t* out = (int64_t*)calloc((size_t)len, sizeof(int64_t));

    if (items == NULL || out == NULL) {
      free(items);
      free(out);
      fprintf(stderr, "allocation failed for %" PRId32 " items\n", len);
      return 1;
    }

    fill_items(items, len);

    if (calc_items(items, len, out) != 0) {
      fprintf(stderr, "warmup calc_items failed for %" PRId32 " items\n", len);
      free(items);
      free(out);
      return 1;
    }

    for (int32_t i = 0; i < len; i += 1) {
      out[i] = 0;
    }

    const uint64_t start = now_ns();
    const int32_t status = calc_items(items, len, out);
    const uint64_t elapsed = now_ns() - start;

    if (status != 0) {
      fprintf(stderr, "calc_items returned %" PRId32 " for %" PRId32 " items\n", status, len);
      free(items);
      free(out);
      return 1;
    }

    printf("%" PRId32 " items: %.3f ms (checksum=%" PRId64 ")\n", len, (double)elapsed / 1000000.0, checksum(out, len));

    free(items);
    free(out);
  }

  return 0;
}
