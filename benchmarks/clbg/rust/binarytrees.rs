// The Computer Language Benchmarks Game
// https://salsa.debian.org/benchmarksgame-team/benchmarksgame/
//
// contributed by the Rust Project Developers

enum Tree {
    Nil,
    Node(Box<Tree>, Box<Tree>),
}

fn item_check(tree: &Tree) -> i32 {
    match tree {
        Tree::Nil => 0,
        Tree::Node(left, right) => 1 + item_check(left) + item_check(right),
    }
}

fn bottom_up_tree(depth: i32) -> Tree {
    if depth > 0 {
        Tree::Node(
            Box::new(bottom_up_tree(depth - 1)),
            Box::new(bottom_up_tree(depth - 1)),
        )
    } else {
        Tree::Node(Box::new(Tree::Nil), Box::new(Tree::Nil))
    }
}

fn main() {
    let n: i32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

    let min_depth = 4;
    let max_depth = if min_depth + 2 > n { min_depth + 2 } else { n };

    {
        let stretch_depth = max_depth + 1;
        let stretch_tree = bottom_up_tree(stretch_depth);
        println!(
            "stretch tree of depth {}\t check: {}",
            stretch_depth,
            item_check(&stretch_tree)
        );
    }

    let long_lived_tree = bottom_up_tree(max_depth);

    for depth in (min_depth..=max_depth).step_by(2) {
        let iterations = 1 << (max_depth - depth + min_depth);
        let mut check = 0;
        for _ in 0..iterations {
            let temp_tree = bottom_up_tree(depth);
            check += item_check(&temp_tree);
        }
        println!("{}\t trees of depth {}\t check: {}", iterations, depth, check);
    }

    println!(
        "long lived tree of depth {}\t check: {}",
        max_depth,
        item_check(&long_lived_tree)
    );
}
