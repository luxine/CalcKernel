#include "ckc_runtime.h"

static uint32_t format_u64(uint64_t value, uint8_t *buffer) {
  uint8_t reverse[20];
  uint32_t count = 0;
  do {
    const uint64_t quotient = value / 10u;
    reverse[count++] = (uint8_t)('0' + (value - quotient * 10u));
    value = quotient;
  } while (value != 0);
  for (uint32_t index = 0; index < count; ++index) {
    buffer[index] = reverse[count - index - 1u];
  }
  return count;
}

static void print_signed(int64_t value) {
  uint8_t buffer[21];
  uint32_t offset = 0;
  uint64_t magnitude;
  if (value < 0) {
    buffer[offset++] = '-';
    magnitude = (uint64_t)(-(value + 1)) + 1u;
  } else {
    magnitude = (uint64_t)value;
  }
  offset += format_u64(magnitude, buffer + offset);
  __ck_runtime_write_stdout(buffer, offset);
}

void __ck_print_i32(int32_t value) { print_signed(value); }
void __ck_print_i64(int64_t value) { print_signed(value); }

void __ck_print_u32(uint32_t value) {
  uint8_t buffer[10];
  const uint32_t length = format_u64(value, buffer);
  __ck_runtime_write_stdout(buffer, length);
}

void __ck_print_u64(uint64_t value) {
  uint8_t buffer[20];
  const uint32_t length = format_u64(value, buffer);
  __ck_runtime_write_stdout(buffer, length);
}

void __ck_print_bool(bool value) {
  static const uint8_t TRUE_BYTES[] = "true";
  static const uint8_t FALSE_BYTES[] = "false";
  if (value) {
    __ck_runtime_write_stdout(TRUE_BYTES, sizeof(TRUE_BYTES) - 1u);
  } else {
    __ck_runtime_write_stdout(FALSE_BYTES, sizeof(FALSE_BYTES) - 1u);
  }
}

void __ck_print_newline(void) {
  const uint8_t newline = '\n';
  __ck_runtime_write_stdout(&newline, 1);
}
