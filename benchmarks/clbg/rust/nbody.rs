// The Computer Language Benchmarks Game
// https://salsa.debian.org/benchmarksgame-team/benchmarksgame/
//
// contributed by the Rust Project Developers
// modified by Cristi Cobzarenco

use std::f64::consts::PI;

const SOLAR_MASS: f64 = 4.0 * PI * PI;
const DAYS_PER_YEAR: f64 = 365.24;

#[derive(Clone, Copy)]
struct Body {
    x: [f64; 3],
    v: [f64; 3],
    mass: f64,
}

const BODIES: [Body; 5] = [
    // Sun
    Body {
        x: [0.0, 0.0, 0.0],
        v: [0.0, 0.0, 0.0],
        mass: SOLAR_MASS,
    },
    // Jupiter
    Body {
        x: [4.84143144246472090e+00, -1.16032004402742839e+00, -1.03622044471123109e-01],
        v: [
            1.66007664274403694e-03 * DAYS_PER_YEAR,
            7.69901118419740425e-03 * DAYS_PER_YEAR,
            -6.90460016972063023e-05 * DAYS_PER_YEAR,
        ],
        mass: 9.54791938424326609e-04 * SOLAR_MASS,
    },
    // Saturn
    Body {
        x: [8.34336671824457987e+00, 4.12479856412430479e+00, -4.03523417114321381e-01],
        v: [
            -2.76742510726862411e-03 * DAYS_PER_YEAR,
            4.99852801234917238e-03 * DAYS_PER_YEAR,
            2.30417297573763929e-05 * DAYS_PER_YEAR,
        ],
        mass: 2.85885980666130812e-04 * SOLAR_MASS,
    },
    // Uranus
    Body {
        x: [1.28943695621391310e+01, -1.51111514016986312e+01, -2.23307578892655734e-01],
        v: [
            2.96460137564761618e-03 * DAYS_PER_YEAR,
            2.37847173959480950e-03 * DAYS_PER_YEAR,
            -2.96589568540237556e-05 * DAYS_PER_YEAR,
        ],
        mass: 4.36624404335156298e-05 * SOLAR_MASS,
    },
    // Neptune
    Body {
        x: [1.53796971148509165e+01, -2.59193146099879641e+01, 1.79258772950371181e-01],
        v: [
            2.68067772490389322e-03 * DAYS_PER_YEAR,
            1.62824170038242295e-03 * DAYS_PER_YEAR,
            -9.51592254519715870e-05 * DAYS_PER_YEAR,
        ],
        mass: 5.15138902046611451e-05 * SOLAR_MASS,
    },
];

fn offset_momentum(bodies: &mut [Body]) {
    let (px, py, pz) = bodies.iter().fold((0.0, 0.0, 0.0), |(px, py, pz), b| {
        (px + b.v[0] * b.mass, py + b.v[1] * b.mass, pz + b.v[2] * b.mass)
    });
    let sun = &mut bodies[0];
    sun.v[0] = -px / SOLAR_MASS;
    sun.v[1] = -py / SOLAR_MASS;
    sun.v[2] = -pz / SOLAR_MASS;
}

fn advance(bodies: &mut [Body], dt: f64) {
    let n = bodies.len();
    for i in 0..n {
        for j in (i + 1)..n {
            let dx = bodies[i].x[0] - bodies[j].x[0];
            let dy = bodies[i].x[1] - bodies[j].x[1];
            let dz = bodies[i].x[2] - bodies[j].x[2];
            let dist = (dx * dx + dy * dy + dz * dz).sqrt();
            let mag = dt / (dist * dist * dist);

            bodies[i].v[0] -= dx * bodies[j].mass * mag;
            bodies[i].v[1] -= dy * bodies[j].mass * mag;
            bodies[i].v[2] -= dz * bodies[j].mass * mag;
            bodies[j].v[0] += dx * bodies[i].mass * mag;
            bodies[j].v[1] += dy * bodies[i].mass * mag;
            bodies[j].v[2] += dz * bodies[i].mass * mag;
        }
    }
    for body in bodies.iter_mut() {
        body.x[0] += dt * body.v[0];
        body.x[1] += dt * body.v[1];
        body.x[2] += dt * body.v[2];
    }
}

fn energy(bodies: &[Body]) -> f64 {
    let mut e = 0.0;
    let n = bodies.len();
    for i in 0..n {
        let b = &bodies[i];
        e += 0.5 * b.mass * (b.v[0] * b.v[0] + b.v[1] * b.v[1] + b.v[2] * b.v[2]);
        for j in (i + 1)..n {
            let dx = b.x[0] - bodies[j].x[0];
            let dy = b.x[1] - bodies[j].x[1];
            let dz = b.x[2] - bodies[j].x[2];
            let dist = (dx * dx + dy * dy + dz * dz).sqrt();
            e -= (b.mass * bodies[j].mass) / dist;
        }
    }
    e
}

fn main() {
    let n: i32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);

    let mut bodies = BODIES;
    offset_momentum(&mut bodies);
    println!("{:.9}", energy(&bodies));

    for _ in 0..n {
        advance(&mut bodies, 0.01);
    }

    println!("{:.9}", energy(&bodies));
}
