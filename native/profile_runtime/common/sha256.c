typedef struct CkProfileSha256 {
  uint32_t state[8];
  uint64_t bytes;
  uint8_t block[64];
  uint32_t used;
} CkProfileSha256;

static uint32_t ck_sha_rotr(uint32_t value, uint32_t count) {
  return (value >> count) | (value << (32u - count));
}

static uint32_t ck_sha_choose(uint32_t x, uint32_t y, uint32_t z) {
  return (x & y) ^ (~x & z);
}

static uint32_t ck_sha_majority(uint32_t x, uint32_t y, uint32_t z) {
  return (x & y) ^ (x & z) ^ (y & z);
}

static void ck_sha_transform(CkProfileSha256 *sha, const uint8_t block[64]) {
  static const uint32_t constants[64] = {
      0x428a2f98u, 0x71374491u, 0xb5c0fbcfu, 0xe9b5dba5u, 0x3956c25bu,
      0x59f111f1u, 0x923f82a4u, 0xab1c5ed5u, 0xd807aa98u, 0x12835b01u,
      0x243185beu, 0x550c7dc3u, 0x72be5d74u, 0x80deb1feu, 0x9bdc06a7u,
      0xc19bf174u, 0xe49b69c1u, 0xefbe4786u, 0x0fc19dc6u, 0x240ca1ccu,
      0x2de92c6fu, 0x4a7484aau, 0x5cb0a9dcu, 0x76f988dau, 0x983e5152u,
      0xa831c66du, 0xb00327c8u, 0xbf597fc7u, 0xc6e00bf3u, 0xd5a79147u,
      0x06ca6351u, 0x14292967u, 0x27b70a85u, 0x2e1b2138u, 0x4d2c6dfcu,
      0x53380d13u, 0x650a7354u, 0x766a0abbu, 0x81c2c92eu, 0x92722c85u,
      0xa2bfe8a1u, 0xa81a664bu, 0xc24b8b70u, 0xc76c51a3u, 0xd192e819u,
      0xd6990624u, 0xf40e3585u, 0x106aa070u, 0x19a4c116u, 0x1e376c08u,
      0x2748774cu, 0x34b0bcb5u, 0x391c0cb3u, 0x4ed8aa4au, 0x5b9cca4fu,
      0x682e6ff3u, 0x748f82eeu, 0x78a5636fu, 0x84c87814u, 0x8cc70208u,
      0x90befffau, 0xa4506cebu, 0xbef9a3f7u, 0xc67178f2u};
  uint32_t words[64];
  for (uint32_t index = 0; index < 16; ++index) {
    const uint32_t offset = index * 4u;
    words[index] = ((uint32_t)block[offset] << 24u) |
                   ((uint32_t)block[offset + 1u] << 16u) |
                   ((uint32_t)block[offset + 2u] << 8u) |
                   (uint32_t)block[offset + 3u];
  }
  for (uint32_t index = 16; index < 64; ++index) {
    const uint32_t x = words[index - 15u];
    const uint32_t y = words[index - 2u];
    const uint32_t s0 = ck_sha_rotr(x, 7) ^ ck_sha_rotr(x, 18) ^ (x >> 3u);
    const uint32_t s1 = ck_sha_rotr(y, 17) ^ ck_sha_rotr(y, 19) ^ (y >> 10u);
    words[index] = words[index - 16u] + s0 + words[index - 7u] + s1;
  }
  uint32_t a = sha->state[0];
  uint32_t b = sha->state[1];
  uint32_t c = sha->state[2];
  uint32_t d = sha->state[3];
  uint32_t e = sha->state[4];
  uint32_t f = sha->state[5];
  uint32_t g = sha->state[6];
  uint32_t h = sha->state[7];
  for (uint32_t index = 0; index < 64; ++index) {
    const uint32_t sum1 = ck_sha_rotr(e, 6) ^ ck_sha_rotr(e, 11) ^ ck_sha_rotr(e, 25);
    const uint32_t first = h + sum1 + ck_sha_choose(e, f, g) + constants[index] + words[index];
    const uint32_t sum0 = ck_sha_rotr(a, 2) ^ ck_sha_rotr(a, 13) ^ ck_sha_rotr(a, 22);
    const uint32_t second = sum0 + ck_sha_majority(a, b, c);
    h = g;
    g = f;
    f = e;
    e = d + first;
    d = c;
    c = b;
    b = a;
    a = first + second;
  }
  sha->state[0] += a;
  sha->state[1] += b;
  sha->state[2] += c;
  sha->state[3] += d;
  sha->state[4] += e;
  sha->state[5] += f;
  sha->state[6] += g;
  sha->state[7] += h;
}

static void ck_sha_init(CkProfileSha256 *sha) {
  static const uint32_t initial[8] = {0x6a09e667u, 0xbb67ae85u, 0x3c6ef372u,
                                      0xa54ff53au, 0x510e527fu, 0x9b05688cu,
                                      0x1f83d9abu, 0x5be0cd19u};
  for (uint32_t index = 0; index < 8; ++index) {
    sha->state[index] = initial[index];
  }
  sha->bytes = 0;
  sha->used = 0;
}

static void ck_sha_update(CkProfileSha256 *sha, const uint8_t *bytes,
                          uint64_t length) {
  sha->bytes += length;
  while (length != 0u) {
    uint32_t available = 64u - sha->used;
    uint32_t take = length < (uint64_t)available ? (uint32_t)length : available;
    for (uint32_t index = 0; index < take; ++index) {
      sha->block[sha->used + index] = bytes[index];
    }
    sha->used += take;
    bytes += take;
    length -= take;
    if (sha->used == 64u) {
      ck_sha_transform(sha, sha->block);
      sha->used = 0;
    }
  }
}

static void ck_sha_finish(CkProfileSha256 *sha, uint8_t output[32]) {
  const uint64_t bits = sha->bytes * 8u;
  sha->block[sha->used++] = 0x80u;
  if (sha->used > 56u) {
    while (sha->used < 64u) {
      sha->block[sha->used++] = 0;
    }
    ck_sha_transform(sha, sha->block);
    sha->used = 0;
  }
  while (sha->used < 56u) {
    sha->block[sha->used++] = 0;
  }
  for (uint32_t index = 0; index < 8; ++index) {
    sha->block[56u + index] = (uint8_t)(bits >> (56u - index * 8u));
  }
  ck_sha_transform(sha, sha->block);
  for (uint32_t index = 0; index < 8; ++index) {
    output[index * 4u] = (uint8_t)(sha->state[index] >> 24u);
    output[index * 4u + 1u] = (uint8_t)(sha->state[index] >> 16u);
    output[index * 4u + 2u] = (uint8_t)(sha->state[index] >> 8u);
    output[index * 4u + 3u] = (uint8_t)sha->state[index];
  }
}
