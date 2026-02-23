/* The Computer Language Benchmarks Game
   https://salsa.debian.org/benchmarksgame-team/benchmarksgame/

   contributed by Mark C. Lewis
   modified slightly by Chad Whipkey
   converted from Java to C by Joseph Piché
   modified by Christoph Bauer
*/

#include <math.h>
#include <stdio.h>
#include <stdlib.h>

#define PI 3.141592653589793
#define SOLAR_MASS (4 * PI * PI)
#define DAYS_PER_YEAR 365.24

struct body {
    double x, y, z;
    double vx, vy, vz;
    double mass;
};

struct body bodies[5];

void init_bodies() {
    // Sun
    bodies[0].x = 0;
    bodies[0].y = 0;
    bodies[0].z = 0;
    bodies[0].vx = 0;
    bodies[0].vy = 0;
    bodies[0].vz = 0;
    bodies[0].mass = SOLAR_MASS;

    // Jupiter
    bodies[1].x = 4.84143144246472090e+00;
    bodies[1].y = -1.16032004402742839e+00;
    bodies[1].z = -1.03622044471123109e-01;
    bodies[1].vx = 1.66007664274403694e-03 * DAYS_PER_YEAR;
    bodies[1].vy = 7.69901118419740425e-03 * DAYS_PER_YEAR;
    bodies[1].vz = -6.90460016972063023e-05 * DAYS_PER_YEAR;
    bodies[1].mass = 9.54791938424326609e-04 * SOLAR_MASS;

    // Saturn
    bodies[2].x = 8.34336671824457987e+00;
    bodies[2].y = 4.12479856412430479e+00;
    bodies[2].z = -4.03523417114321381e-01;
    bodies[2].vx = -2.76742510726862411e-03 * DAYS_PER_YEAR;
    bodies[2].vy = 4.99852801234917238e-03 * DAYS_PER_YEAR;
    bodies[2].vz = 2.30417297573763929e-05 * DAYS_PER_YEAR;
    bodies[2].mass = 2.85885980666130812e-04 * SOLAR_MASS;

    // Uranus
    bodies[3].x = 1.28943695621391310e+01;
    bodies[3].y = -1.51111514016986312e+01;
    bodies[3].z = -2.23307578892655734e-01;
    bodies[3].vx = 2.96460137564761618e-03 * DAYS_PER_YEAR;
    bodies[3].vy = 2.37847173959480950e-03 * DAYS_PER_YEAR;
    bodies[3].vz = -2.96589568540237556e-05 * DAYS_PER_YEAR;
    bodies[3].mass = 4.36624404335156298e-05 * SOLAR_MASS;

    // Neptune
    bodies[4].x = 1.53796971148509165e+01;
    bodies[4].y = -2.59193146099879641e+01;
    bodies[4].z = 1.79258772950371181e-01;
    bodies[4].vx = 2.68067772490389322e-03 * DAYS_PER_YEAR;
    bodies[4].vy = 1.62824170038242295e-03 * DAYS_PER_YEAR;
    bodies[4].vz = -9.51592254519715870e-05 * DAYS_PER_YEAR;
    bodies[4].mass = 5.15138902046611451e-05 * SOLAR_MASS;
}

void offset_momentum() {
    double px = 0, py = 0, pz = 0;
    for (int i = 0; i < 5; i++) {
        px += bodies[i].vx * bodies[i].mass;
        py += bodies[i].vy * bodies[i].mass;
        pz += bodies[i].vz * bodies[i].mass;
    }
    bodies[0].vx = -px / SOLAR_MASS;
    bodies[0].vy = -py / SOLAR_MASS;
    bodies[0].vz = -pz / SOLAR_MASS;
}

void advance(double dt) {
    for (int i = 0; i < 5; i++) {
        for (int j = i + 1; j < 5; j++) {
            double dx = bodies[i].x - bodies[j].x;
            double dy = bodies[i].y - bodies[j].y;
            double dz = bodies[i].z - bodies[j].z;
            double dist = sqrt(dx * dx + dy * dy + dz * dz);
            double mag = dt / (dist * dist * dist);
            bodies[i].vx -= dx * bodies[j].mass * mag;
            bodies[i].vy -= dy * bodies[j].mass * mag;
            bodies[i].vz -= dz * bodies[j].mass * mag;
            bodies[j].vx += dx * bodies[i].mass * mag;
            bodies[j].vy += dy * bodies[i].mass * mag;
            bodies[j].vz += dz * bodies[i].mass * mag;
        }
    }
    for (int i = 0; i < 5; i++) {
        bodies[i].x += dt * bodies[i].vx;
        bodies[i].y += dt * bodies[i].vy;
        bodies[i].z += dt * bodies[i].vz;
    }
}

double energy() {
    double e = 0.0;
    for (int i = 0; i < 5; i++) {
        e += 0.5 * bodies[i].mass *
             (bodies[i].vx * bodies[i].vx +
              bodies[i].vy * bodies[i].vy +
              bodies[i].vz * bodies[i].vz);
        for (int j = i + 1; j < 5; j++) {
            double dx = bodies[i].x - bodies[j].x;
            double dy = bodies[i].y - bodies[j].y;
            double dz = bodies[i].z - bodies[j].z;
            double dist = sqrt(dx * dx + dy * dy + dz * dz);
            e -= (bodies[i].mass * bodies[j].mass) / dist;
        }
    }
    return e;
}

int main(int argc, char *argv[]) {
    int n = argc > 1 ? atoi(argv[1]) : 1000;

    init_bodies();
    offset_momentum();
    printf("%.9f\n", energy());

    for (int i = 0; i < n; i++) {
        advance(0.01);
    }

    printf("%.9f\n", energy());
    return 0;
}
