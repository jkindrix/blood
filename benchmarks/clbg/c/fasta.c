/* The Computer Language Benchmarks Game
   https://salsa.debian.org/benchmarksgame-team/benchmarksgame/

   contributed by David Pyke
   modified by Rolf Schroedter
*/

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define IM 139968
#define IA 3877
#define IC 29573

#define LINE_LENGTH 60

static int last = 42;

static double gen_random(double max) {
    return max * (last = (last * IA + IC) % IM) / IM;
}

struct aminoacid {
    char c;
    double p;
};

static void make_cumulative(struct aminoacid *genelist, int count) {
    double cp = 0.0;
    for (int i = 0; i < count; i++) {
        cp += genelist[i].p;
        genelist[i].p = cp;
    }
}

static char select_random(struct aminoacid *genelist, int count) {
    double r = gen_random(1.0);
    for (int i = 0; i < count; i++) {
        if (r < genelist[i].p) return genelist[i].c;
    }
    return genelist[count - 1].c;
}

static void make_random_fasta(const char *id, const char *desc,
                              struct aminoacid *genelist, int count, int n) {
    printf(">%s %s\n", id, desc);

    char line[LINE_LENGTH + 1];
    line[LINE_LENGTH] = '\0';

    int pos = 0;
    while (n > 0) {
        int len = n < LINE_LENGTH ? n : LINE_LENGTH;
        for (int i = 0; i < len; i++) {
            line[i] = select_random(genelist, count);
        }
        line[len] = '\0';
        puts(line);
        n -= len;
    }
}

static void make_repeat_fasta(const char *id, const char *desc,
                              const char *alu, int n) {
    printf(">%s %s\n", id, desc);

    int alu_len = strlen(alu);
    char line[LINE_LENGTH + 1];
    line[LINE_LENGTH] = '\0';

    int alu_pos = 0;
    while (n > 0) {
        int len = n < LINE_LENGTH ? n : LINE_LENGTH;
        for (int i = 0; i < len; i++) {
            line[i] = alu[alu_pos];
            alu_pos = (alu_pos + 1) % alu_len;
        }
        line[len] = '\0';
        puts(line);
        n -= len;
    }
}

static struct aminoacid iub[] = {
    {'a', 0.27}, {'c', 0.12}, {'g', 0.12}, {'t', 0.27},
    {'B', 0.02}, {'D', 0.02}, {'H', 0.02}, {'K', 0.02},
    {'M', 0.02}, {'N', 0.02}, {'R', 0.02}, {'S', 0.02},
    {'V', 0.02}, {'W', 0.02}, {'Y', 0.02}
};
#define IUB_COUNT (sizeof(iub) / sizeof(iub[0]))

static struct aminoacid homosapiens[] = {
    {'a', 0.3029549426680}, {'c', 0.1979883004921},
    {'g', 0.1975473066391}, {'t', 0.3015094502008}
};
#define HOMOSAPIENS_COUNT (sizeof(homosapiens) / sizeof(homosapiens[0]))

static const char alu[] =
    "GGCCGGGCGCGGTGGCTCACGCCTGTAATCCCAGCACTTTGG"
    "GAGGCCGAGGCGGGCGGATCACCTGAGGTCAGGAGTTCGAGA"
    "CCAGCCTGGCCAACATGGTGAAACCCCGTCTCTACTAAAAAT"
    "ACAAAAATTAGCCGGGCGTGGTGGCGCGCGCCTGTAATCCCA"
    "GCTACTCGGGAGGCTGAGGCAGGAGAATCGCTTGAACCCGGG"
    "AGGCGGAGGTTGCAGTGAGCCGAGATCGCGCCACTGCACTCC"
    "AGCCTGGGCGACAGAGCGAGACTCCGTCTCAAAAA";

int main(int argc, char *argv[]) {
    int n = argc > 1 ? atoi(argv[1]) : 1000;

    make_cumulative(iub, IUB_COUNT);
    make_cumulative(homosapiens, HOMOSAPIENS_COUNT);

    make_repeat_fasta("ONE", "Homo sapiens alu", alu, n * 2);
    make_random_fasta("TWO", "IUB ambiguity codes", iub, IUB_COUNT, n * 3);
    make_random_fasta("THREE", "Homo sapiens frequency", homosapiens, HOMOSAPIENS_COUNT, n * 5);

    return 0;
}
