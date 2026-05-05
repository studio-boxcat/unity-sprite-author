// Run: cargo run --release --example probe_formulas
//
// Hardcase fixtures: (rect_w, rect_h, pivot_x, pivot_y, orig_pivot_x_or_NaN,
//   orig_pivot_y_or_NaN, target_off_x_bits, target_off_y_bits).
// Source of truth: target bits read directly from .asset YAML strings.
// Try a battery of formulas; report which (if any) reproduces every fixture.

#[derive(Debug, Clone, Copy)]
struct Case {
    name: &'static str,
    w: f32,
    h: f32,
    px: f32,
    py: f32,
    opx: f32, // tps original pivot x (NaN if unknown)
    opy: f32, // tps original pivot y
    bits_x: u32,
    bits_y: u32,
}

const CASES: &[Case] = &[
    Case {
        name: "AC_IC_Orgel",
        w: 103.0,
        h: 92.0,
        px: 0.485437,
        py: 0.5,
        opx: 0.485437,
        opy: 0.5,
        bits_x: 0xbfbfffa0,
        bits_y: 0x00000000,
    },
    Case {
        name: "AC_PT_Icon_Gift",
        w: 38.0,
        h: 81.0,
        px: 0.0,
        py: 0.45726,
        opx: 0.0,
        opy: 0.54274,
        bits_x: 0xc1980000,
        bits_y: 0xc05d9080,
    },
    Case {
        name: "AC_Platform_Apple",
        w: 75.0,
        h: 76.0,
        px: 0.51933336,
        py: 0.5125,
        opx: 0.5125,
        opy: 0.4875,
        bits_x: 0x3fb999a0,
        bits_y: 0x3f733300,
    },
    Case {
        name: "OE_Calendar",
        w: 28.0,
        h: 75.0,
        px: 0.0,
        py: 0.653333,
        opx: 0.0,
        opy: 0.346667,
        bits_x: 0xc1600000,
        bits_y: 0x4137ffe0,
    },
    Case {
        name: "OE_Icon_Sun",
        w: 53.0,
        h: 102.0,
        px: 0.0,
        py: 0.470588,
        opx: 0.0,
        opy: 0.529412,
        bits_x: 0xc1d40000,
        bits_y: 0xc0400080,
    },
    Case {
        name: "OA_DC_Autumn2",
        w: 40.0,
        h: 78.0,
        px: 0.0,
        py: 0.381443,
        opx: 0.0,
        opy: 0.618557,
        bits_x: 0xc1a00000,
        bits_y: 0xc113f590,
    },
    Case {
        name: "OA_Lock",
        w: 43.0,
        h: 115.0,
        px: 0.0,
        py: 0.817391,
        opx: 0.0,
        opy: 0.182609,
        bits_x: 0xc1ac0000,
        bits_y: 0x4211fff8,
    },
    Case {
        name: "OA_ArrowBrown",
        w: 42.0,
        h: 92.0,
        px: 1.0,
        py: 0.586957,
        opx: 1.0,
        opy: 0.413043,
        bits_x: 0x41a80000,
        bits_y: 0x41000030,
    },
    Case {
        name: "OA_ArrowWhite",
        w: 33.0,
        h: 43.0,
        px: 0.0,
        py: 0.139535,
        opx: 0.0,
        opy: 0.860465,
        bits_x: 0xc1840000,
        bits_y: 0xc1780000,
    },
];

type Formula = fn(/*size*/ f32, /*pivot*/ f32, /*orig_pivot*/ f32) -> f32;

fn f_a(s: f32, p: f32, _o: f32) -> f32 {
    p * s - s * 0.5
}
fn f_b(s: f32, p: f32, _o: f32) -> f32 {
    (p - 0.5) * s
}
fn f_c(s: f32, _p: f32, o: f32) -> f32 {
    (0.5 - o) * s
}
fn f_d(s: f32, _p: f32, o: f32) -> f32 {
    s * 0.5 - o * s
}
fn f_e(s: f32, _p: f32, o: f32) -> f32 {
    s * (0.5 - o)
}
fn f_f(s: f32, _p: f32, o: f32) -> f32 {
    -(o - 0.5) * s
}
fn f_g(s: f32, _p: f32, o: f32) -> f32 {
    s * 0.5 - s * o
}
fn f_h(s: f32, _p: f32, o: f32) -> f32 {
    -o * s + s * 0.5
}
// Original pivot in f64 and cast back
fn f_i(s: f32, _p: f32, o: f32) -> f32 {
    ((0.5 - o as f64) * s as f64) as f32
}
fn f_j(s: f32, _p: f32, o: f32) -> f32 {
    (s as f64 * 0.5 - o as f64 * s as f64) as f32
}
// FMA variants on the original pivot
fn f_k(s: f32, _p: f32, o: f32) -> f32 {
    (-o).mul_add(s, s * 0.5)
}
fn f_l(s: f32, _p: f32, o: f32) -> f32 {
    o.mul_add(-s, s * 0.5)
}

const FORMULAS: &[(&str, Formula)] = &[
    ("p*s - s*.5     [current]", f_a),
    ("(p-.5)*s", f_b),
    ("(.5 - o)*s", f_c),
    ("s*.5 - o*s", f_d),
    ("s*(.5 - o)", f_e),
    ("-(o-.5)*s", f_f),
    ("s*.5 - s*o", f_g),
    ("-o*s + s*.5", f_h),
    ("f64 (.5 - o)*s -> f32", f_i),
    ("f64 s*.5 - o*s -> f32", f_j),
    ("(-o).mul_add(s, s*.5)", f_k),
    ("o.mul_add(-s, s*.5)", f_l),
];

fn main() {
    // Per-case Y-axis bit dump for AC_PT_Icon_Gift, OE_Calendar, OA_DC_Autumn2.
    // Some have orig.y data; some don't. Dump all formulas to find a pattern.
    let dump_cases: &[&str] = &[
        "AC_PT_Icon_Gift",
        "OE_Calendar",
        "OE_Icon_Sun",
        "OA_DC_Autumn2",
        "OA_Lock",
        "OA_ArrowBrown",
        "OA_ArrowWhite",
        "AC_Platform_Apple",
    ];
    for c in CASES {
        if !dump_cases.contains(&c.name) {
            continue;
        }
        println!("\n=== {} (h={}, py={}, opy={}) target=0x{:08x} ({}) ===",
            c.name, c.h, c.py, c.opy, c.bits_y, f32::from_bits(c.bits_y));
        for (name, f) in FORMULAS {
            let got = f(c.h, c.py, c.opy);
            let mark = if got.to_bits() == c.bits_y { "✓" } else { " " };
            println!(
                "  {} {:>30}: 0x{:08x} ({})",
                mark,
                name,
                got.to_bits(),
                got
            );
        }
    }

    println!("\n");
    println!(
        "{:>30}  ┃ {:^10} ┃ {}",
        "formula",
        "x match",
        "y match (target ┃ result : sprite)"
    );
    for (name, f) in FORMULAS {
        let mut x_match = 0;
        let mut y_match = 0;
        let mut x_total = 0;
        let mut y_total = 0;
        let mut first_y_fail: Option<&Case> = None;
        let mut first_y_fail_got = 0u32;
        for c in CASES {
            // x
            x_total += 1;
            let got_x = f(c.w, c.px, c.opx);
            if got_x.to_bits() == c.bits_x {
                x_match += 1;
            }
            // y
            y_total += 1;
            let got_y = f(c.h, c.py, c.opy);
            if got_y.to_bits() == c.bits_y {
                y_match += 1;
            } else if first_y_fail.is_none() && c.bits_y != 0 {
                first_y_fail = Some(c);
                first_y_fail_got = got_y.to_bits();
            }
        }
        let example = match first_y_fail {
            Some(c) => format!(
                "0x{:08x} ┃ 0x{:08x} : {}",
                c.bits_y, first_y_fail_got, c.name
            ),
            None => "all match".into(),
        };
        println!(
            "{:>30}  ┃ {:>3}/{:<3}  ┃ {:>3}/{:<3}  ┃ {}",
            name, x_match, x_total, y_match, y_total, example
        );
    }
}
