#include "ckc_runtime.h"

#define CKC_RYU_NO_MALLOC 1
#include "ryu/ryu.h"

#if defined(_MSC_VER)
/* MSVC references this marker for floating-point translation units even when
   no CRT floating-point helper is used. The freestanding runtime owns it. */
int _fltused = 0;
#endif

static uint64_t f64_bits(double value) {
  union {
    double floating;
    uint64_t bits;
  } representation;
  representation.floating = value;
  return representation.bits;
}

static uint32_t copy_bytes(uint8_t *output, const char *input,
                           uint32_t length) {
  for (uint32_t index = 0; index < length; ++index) {
    output[index] = (uint8_t)input[index];
  }
  return length;
}

static uint32_t decimal_digits(uint32_t value) {
  uint32_t digits = 1;
  while (value >= 10u) {
    value /= 10u;
    ++digits;
  }
  return digits;
}

static uint32_t write_exponent(uint8_t *output, int32_t exponent) {
  uint32_t index = 0;
  output[index++] = 'e';
  uint32_t magnitude;
  if (exponent < 0) {
    output[index++] = '-';
    magnitude = (uint32_t)(-exponent);
  } else {
    magnitude = (uint32_t)exponent;
  }
  uint8_t reverse[3];
  uint32_t count = 0;
  do {
    reverse[count++] = (uint8_t)('0' + magnitude % 10u);
    magnitude /= 10u;
  } while (magnitude != 0u);
  while (count != 0u) {
    output[index++] = reverse[--count];
  }
  return index;
}

static uint32_t normalize_ryu(const char *ryu, uint32_t length,
                              uint8_t *output) {
  uint32_t offset = 0;
  if (ryu[0] == '-') {
    output[offset++] = '-';
    ++ryu;
    --length;
  }
  uint32_t exponent_at = 0;
  while (exponent_at < length && ryu[exponent_at] != 'E') {
    ++exponent_at;
  }
  char digits[17];
  uint32_t digit_count = 0;
  for (uint32_t index = 0; index < exponent_at; ++index) {
    if (ryu[index] != '.') {
      digits[digit_count++] = ryu[index];
    }
  }
  int32_t exponent = 0;
  bool negative_exponent = false;
  for (uint32_t index = exponent_at + 1u; index < length; ++index) {
    if (ryu[index] == '-') {
      negative_exponent = true;
    } else {
      exponent = exponent * 10 + (int32_t)(ryu[index] - '0');
    }
  }
  if (negative_exponent) {
    exponent = -exponent;
  }

  const int32_t decimal_position = exponent + 1;
  uint32_t fixed_length;
  if (decimal_position <= 0) {
    fixed_length = 2u + (uint32_t)(-decimal_position) + digit_count;
  } else if ((uint32_t)decimal_position >= digit_count) {
    fixed_length = (uint32_t)decimal_position;
  } else {
    fixed_length = digit_count + 1u;
  }
  const uint32_t exponent_magnitude =
      (uint32_t)(exponent < 0 ? -exponent : exponent);
  const uint32_t scientific_length =
      digit_count + (digit_count > 1u ? 1u : 0u) + 1u +
      (exponent < 0 ? 1u : 0u) + decimal_digits(exponent_magnitude);

  if (fixed_length <= scientific_length) {
    if (decimal_position <= 0) {
      output[offset++] = '0';
      output[offset++] = '.';
      for (int32_t zero = 0; zero < -decimal_position; ++zero) {
        output[offset++] = '0';
      }
      offset += copy_bytes(output + offset, digits, digit_count);
    } else if ((uint32_t)decimal_position >= digit_count) {
      offset += copy_bytes(output + offset, digits, digit_count);
      for (uint32_t zero = digit_count; zero < (uint32_t)decimal_position;
           ++zero) {
        output[offset++] = '0';
      }
    } else {
      offset +=
          copy_bytes(output + offset, digits, (uint32_t)decimal_position);
      output[offset++] = '.';
      offset += copy_bytes(output + offset, digits + decimal_position,
                           digit_count - (uint32_t)decimal_position);
    }
    return offset;
  }

  output[offset++] = (uint8_t)digits[0];
  if (digit_count > 1u) {
    output[offset++] = '.';
    offset += copy_bytes(output + offset, digits + 1, digit_count - 1u);
  }
  offset += write_exponent(output + offset, exponent);
  return offset;
}

void __ck_print_f64(double value) {
  const uint64_t bits = f64_bits(value);
  const uint64_t exponent = (bits >> 52u) & 0x7ffu;
  const uint64_t mantissa = bits & 0x000fffffffffffffull;
  const bool negative = (bits >> 63u) != 0u;
  uint8_t output[CKC_RUNTIME_BUFFER_SIZE];
  uint32_t length;
  if (exponent == 0x7ffu) {
    if (mantissa != 0u) {
      length = copy_bytes(output, "nan", 3);
    } else if (negative) {
      length = copy_bytes(output, "-inf", 4);
    } else {
      length = copy_bytes(output, "inf", 3);
    }
  } else if ((bits << 1u) == 0u) {
    length =
        copy_bytes(output, negative ? "-0.0" : "0.0", negative ? 4u : 3u);
  } else {
    char ryu[25];
    const int ryu_length = d2s_buffered_n(value, ryu);
    length = normalize_ryu(ryu, (uint32_t)ryu_length, output);
  }
  __ck_runtime_write_stdout(output, length);
}
