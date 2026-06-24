#define _POSIX_C_SOURCE 200809L

#include "pricing.h"

#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int32_t positive_i32_arg(const char* value, const char* flag) {
  char* end = NULL;
  const long parsed = strtol(value, &end, 10);
  if (value == end || *end != '\0' || parsed <= 0 || parsed > INT32_MAX) {
    fprintf(stderr, "%s must be a positive i32\n", flag);
    exit(2);
  }
  return (int32_t)parsed;
}

static void parse_args(int argc, char** argv, int32_t* items, int32_t* iterations) {
  *items = 100000;
  *iterations = 1000;

  for (int index = 1; index < argc; index += 1) {
    if (strcmp(argv[index], "--items") == 0 && index + 1 < argc) {
      *items = positive_i32_arg(argv[index + 1], "--items");
      index += 1;
    } else if (strcmp(argv[index], "--iterations") == 0 && index + 1 < argc) {
      *iterations = positive_i32_arg(argv[index + 1], "--iterations");
      index += 1;
    } else {
      fprintf(stderr, "unknown option: %s\n", argv[index]);
      exit(2);
    }
  }
}

static void fill_items(Item* items, int32_t len) {
  for (int32_t i = 0; i < len; i += 1) {
    items[i].price = 1000 + (i % 997);
    items[i].qty = 1 + (i % 9);
    items[i].discount = i % 113;
    items[i].tax_rate_ppm = 50000 + (i % 17) * 2500;
  }
}

static int64_t expected_checksum(const Item* items, int32_t len) {
  int64_t total = 0;
  for (int32_t i = 0; i < len; i += 1) {
    const int64_t subtotal = items[i].price * items[i].qty;
    const int64_t after_discount = subtotal - items[i].discount;
    const int64_t tax = after_discount * items[i].tax_rate_ppm / 1000000;
    total += after_discount + tax;
  }
  return total;
}

static int64_t checksum(const int64_t* values, int32_t len) {
  int64_t total = 0;
  for (int32_t i = 0; i < len; i += 1) {
    total += values[i];
  }
  return total;
}

int main(int argc, char** argv) {
  int32_t len = 0;
  int32_t iterations = 0;
  parse_args(argc, argv, &len, &iterations);

  Item* items = (Item*)malloc(sizeof(Item) * (size_t)len);
  int64_t* out = (int64_t*)calloc((size_t)len, sizeof(int64_t));
  if (items == NULL || out == NULL) {
    free(items);
    free(out);
    fprintf(stderr, "allocation failed\n");
    return 1;
  }

  fill_items(items, len);

  for (int32_t iteration = 0; iteration < iterations; iteration += 1) {
    const int32_t status = calc_items(items, len, out);
    if (status != 0) {
      fprintf(stderr, "calc_items returned %" PRId32 "\n", status);
      free(items);
      free(out);
      return 1;
    }
  }

  const int64_t expected = expected_checksum(items, len);
  const int64_t actual = checksum(out, len);
  if (actual != expected) {
    fprintf(stderr, "checksum mismatch: expected %" PRId64 ", got %" PRId64 "\n", expected, actual);
    free(items);
    free(out);
    return 1;
  }

  printf("pricing-c-unchecked items=%" PRId32 " iterations=%" PRId32 " checksum=%" PRId64 "\n", len, iterations, actual);
  free(items);
  free(out);
  return 0;
}
