#include <cstddef>
#include <cstdint>

extern "C" void matmul(const double* a, const double* b, double* out,
                       std::int32_t n) {
    if (n <= 0) {
        return;
    }

    const std::size_t side = static_cast<std::size_t>(n);
    const std::size_t element_count = side * side;
    for (std::size_t index = 0; index < element_count; ++index) {
        out[index] = 0.0;
    }

    for (std::size_t i = 0; i < side; ++i) {
        for (std::size_t k = 0; k < side; ++k) {
            const double left = a[i * side + k];
            const std::size_t output_row = i * side;
            const std::size_t right_row = k * side;
            for (std::size_t j = 0; j < side; ++j) {
                out[output_row + j] += left * b[right_row + j];
            }
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
        const std::size_t top = (row - 1) * side;
        const std::size_t middle = row * side;
        const std::size_t bottom = (row + 1) * side;
        const std::size_t output_row = row * side;
        for (std::size_t column = 1; column + 1 < side; ++column) {
            const double weighted_sum =
                input[top + column - 1] +
                2.0 * input[top + column] +
                input[top + column + 1] +
                2.0 * input[middle + column - 1] +
                4.0 * input[middle + column] +
                2.0 * input[middle + column + 1] +
                input[bottom + column - 1] +
                2.0 * input[bottom + column] +
                input[bottom + column + 1];
            out[output_row + column] = weighted_sum / 16.0;
        }
    }
}
