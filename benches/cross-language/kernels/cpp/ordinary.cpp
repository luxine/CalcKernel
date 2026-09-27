#include <cstddef>
#include <cstdint>

extern "C" void matmul(const double* a, const double* b, double* out,
                       std::int32_t n) {
    if (n <= 0) {
        return;
    }

    const std::size_t side = static_cast<std::size_t>(n);
    for (std::size_t i = 0; i < side; ++i) {
        for (std::size_t j = 0; j < side; ++j) {
            double sum = 0.0;
            for (std::size_t k = 0; k < side; ++k) {
                sum += a[i * side + k] * b[k * side + j];
            }
            out[i * side + j] = sum;
        }
    }
}

extern "C" void convolve(const double* input, double* out,
                         std::int32_t n) {
    if (n <= 0) {
        return;
    }

    const std::size_t side = static_cast<std::size_t>(n);
    const std::size_t element_count = side * side;
    for (std::size_t index = 0; index < element_count; ++index) {
        out[index] = 0.0;
    }

    if (side < 3) {
        return;
    }

    for (std::size_t row = 1; row + 1 < side; ++row) {
        for (std::size_t column = 1; column + 1 < side; ++column) {
            const double weighted_sum =
                input[(row - 1) * side + column - 1] +
                2.0 * input[(row - 1) * side + column] +
                input[(row - 1) * side + column + 1] +
                2.0 * input[row * side + column - 1] +
                4.0 * input[row * side + column] +
                2.0 * input[row * side + column + 1] +
                input[(row + 1) * side + column - 1] +
                2.0 * input[(row + 1) * side + column] +
                input[(row + 1) * side + column + 1];
            out[row * side + column] = weighted_sum / 16.0;
        }
    }
}
