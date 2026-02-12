/* The Computer Language Benchmarks Game
   https://salsa.debian.org/benchmarksgame-team/benchmarksgame/

   contributed by Ledrug
*/

#include <stdio.h>
#include <stdlib.h>
#include <math.h>

double A(int i, int j) {
    return 1.0 / ((i + j) * (i + j + 1) / 2 + i + 1);
}

void Av(int n, double *v, double *out) {
    for (int i = 0; i < n; i++) {
        double sum = 0;
        for (int j = 0; j < n; j++) {
            sum += A(i, j) * v[j];
        }
        out[i] = sum;
    }
}

void Atv(int n, double *v, double *out) {
    for (int i = 0; i < n; i++) {
        double sum = 0;
        for (int j = 0; j < n; j++) {
            sum += A(j, i) * v[j];
        }
        out[i] = sum;
    }
}

void AtAv(int n, double *v, double *out, double *tmp) {
    Av(n, v, tmp);
    Atv(n, tmp, out);
}

int main(int argc, char *argv[]) {
    int n = argc > 1 ? atoi(argv[1]) : 100;

    double *u = malloc(n * sizeof(double));
    double *v = malloc(n * sizeof(double));
    double *tmp = malloc(n * sizeof(double));

    for (int i = 0; i < n; i++) {
        u[i] = 1.0;
    }

    for (int i = 0; i < 10; i++) {
        AtAv(n, u, v, tmp);
        AtAv(n, v, u, tmp);
    }

    double vBv = 0, vv = 0;
    for (int i = 0; i < n; i++) {
        vBv += u[i] * v[i];
        vv += v[i] * v[i];
    }

    printf("%.9f\n", sqrt(vBv / vv));

    free(u);
    free(v);
    free(tmp);
    return 0;
}
