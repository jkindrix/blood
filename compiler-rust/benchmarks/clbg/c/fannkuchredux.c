/* The Computer Language Benchmarks Game
   https://salsa.debian.org/benchmarksgame-team/benchmarksgame/

   contributed by Ledrug
*/

#include <stdio.h>
#include <stdlib.h>

int max_flips = 0;
int checksum = 0;

void fannkuch(int n) {
    int perm[n], perm1[n], count[n];
    int r, flips, i, k, tmp;

    for (i = 0; i < n; i++) perm1[i] = i;

    r = n;
    int nperm = 0;

    while (1) {
        while (r != 1) {
            count[r - 1] = r;
            r--;
        }

        for (i = 0; i < n; i++) perm[i] = perm1[i];

        flips = 0;
        while ((k = perm[0]) != 0) {
            int k2 = (k + 1) >> 1;
            for (i = 0; i < k2; i++) {
                tmp = perm[i];
                perm[i] = perm[k - i];
                perm[k - i] = tmp;
            }
            flips++;
        }

        if (flips > max_flips) max_flips = flips;
        checksum += (nperm & 1) ? -flips : flips;
        nperm++;

        while (1) {
            if (r == n) return;
            int perm0 = perm1[0];
            for (i = 0; i < r; i++) perm1[i] = perm1[i + 1];
            perm1[r] = perm0;
            if (--count[r] > 0) break;
            r++;
        }
    }
}

int main(int argc, char *argv[]) {
    int n = argc > 1 ? atoi(argv[1]) : 7;
    fannkuch(n);
    printf("%d\nPfannkuchen(%d) = %d\n", checksum, n, max_flips);
    return 0;
}
