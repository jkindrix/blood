/* Pass-by-Value Struct Benchmark
 *
 * Tests performance of passing structs by value in tight loops.
 * This is a direct comparison to Blood's val_test.blood benchmark.
 *
 * Expected output: Sum: 3e+07
 */

#include <stdio.h>

typedef struct {
    double x;
    double y;
} Point;

double access_val(Point p) {
    return p.x + p.y;
}

int main() {
    Point pt = {1.0, 2.0};
    double sum = 0.0;
    for (int i = 0; i < 10000000; i++) {
        sum += access_val(pt);
    }
    printf("Sum: %g\n", sum);
    return 0;
}
