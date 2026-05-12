// Ear-clipping triangulator. Port of meow-tower's
// `Core/Runtime/SpriteMeshAuthoring/Triangulator.cs`, which is itself the
// runevision Unity-wiki recipe. Auto-handles input winding via signed-area
// check (negative area ⇒ vertex order reversed). Returns ear-clipped triangle
// indices into the input vertex list.
//
// Used by combine.rs for polygon parts in `.tps.fab.json` manifests.

pub fn triangulate(points: &[[f32; 2]]) -> Vec<u16> {
    let n = points.len();
    if n < 3 {
        return Vec::new();
    }

    // Winding normalization: signed area > 0 ⇒ CCW (use as-is); negative ⇒
    // reverse so the inside-triangle check sees consistent winding.
    let mut ring: Vec<u16> = if signed_area(points) > 0.0 {
        (0..n as u16).collect()
    } else {
        (0..n as u16).rev().collect()
    };

    let mut indices: Vec<u16> = Vec::with_capacity((n - 2) * 3);
    let mut nv = n;
    // Safety bound: each successful ear-clip removes one vertex. 2 * nv slack
    // bails out on bad input (self-intersecting, all-collinear) instead of
    // spinning.
    let mut count = 2 * nv;
    let mut vi = nv - 1;
    while nv > 2 {
        if count == 0 {
            return indices;
        }
        count -= 1;

        let mut u = vi;
        if u >= nv {
            u = 0;
        }
        vi = u + 1;
        if vi >= nv {
            vi = 0;
        }
        let mut w = vi + 1;
        if w >= nv {
            w = 0;
        }

        if snip(points, u, vi, w, nv, &ring) {
            indices.push(ring[u]);
            indices.push(ring[vi]);
            indices.push(ring[w]);
            ring.remove(vi);
            nv -= 1;
            count = 2 * nv;
        }
    }
    indices
}

fn signed_area(points: &[[f32; 2]]) -> f32 {
    let n = points.len();
    let mut a = 0.0f32;
    let mut p = n - 1;
    for q in 0..n {
        let [px, py] = points[p];
        let [qx, qy] = points[q];
        a += px * qy - qx * py;
        p = q;
    }
    a * 0.5
}

fn snip(points: &[[f32; 2]], u: usize, v: usize, w: usize, n: usize, ring: &[u16]) -> bool {
    let a = points[ring[u] as usize];
    let b = points[ring[v] as usize];
    let c = points[ring[w] as usize];
    // C# port's `Mathf.Epsilon > cross` rejects collinear / wrong-wound ears.
    // Mathf.Epsilon (= Single.Epsilon ≈ 1.4e-45) is effectively zero; `<= 0.0`
    // is the same predicate for any cross product above the subnormal range.
    if (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]) <= 0.0 {
        return false;
    }
    for p in 0..n {
        if p == u || p == v || p == w {
            continue;
        }
        let pp = points[ring[p] as usize];
        if inside_triangle(a, b, c, pp) {
            return false;
        }
    }
    true
}

fn inside_triangle(a: [f32; 2], b: [f32; 2], c: [f32; 2], p: [f32; 2]) -> bool {
    let ax = c[0] - b[0];
    let ay = c[1] - b[1];
    let bx = a[0] - c[0];
    let by = a[1] - c[1];
    let cx = b[0] - a[0];
    let cy = b[1] - a[1];
    let apx = p[0] - a[0];
    let apy = p[1] - a[1];
    let bpx = p[0] - b[0];
    let bpy = p[1] - b[1];
    let cpx = p[0] - c[0];
    let cpy = p[1] - c[1];
    let a_cross_bp = ax * bpy - ay * bpx;
    let c_cross_ap = cx * apy - cy * apx;
    let b_cross_cp = bx * cpy - by * cpx;
    a_cross_bp >= 0.0 && b_cross_cp >= 0.0 && c_cross_ap >= 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fewer_than_three_returns_empty() {
        assert!(triangulate(&[]).is_empty());
        assert!(triangulate(&[[0.0, 0.0]]).is_empty());
        assert!(triangulate(&[[0.0, 0.0], [1.0, 0.0]]).is_empty());
    }

    #[test]
    fn triangle_ccw_produces_one_triangle() {
        let idx = triangulate(&[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
        assert_eq!(idx.len(), 3);
        let mut sorted = idx.clone();
        sorted.sort();
        assert_eq!(sorted, vec![0, 1, 2]);
    }

    #[test]
    fn triangle_cw_auto_normalized() {
        let idx = triangulate(&[[0.0, 0.0], [0.0, 1.0], [1.0, 0.0]]);
        assert_eq!(idx.len(), 3);
        let mut sorted = idx.clone();
        sorted.sort();
        assert_eq!(sorted, vec![0, 1, 2]);
    }

    #[test]
    fn square_ccw_produces_two_triangles_covering_all_verts() {
        let pts = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let idx = triangulate(&pts);
        assert_eq!(idx.len(), 6);
        // Triangles together must cover the full square (area = 1).
        assert!((triangle_area_sum(&pts, &idx) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn l_shape_concave_six_verts() {
        // Concave L-shape (CCW):
        //   (0,2)─(1,2)
        //     │     │
        //   (0,1)─(2,1)
        //              │
        //              (2,0)
        let pts = [[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [1.0, 1.0], [1.0, 2.0], [0.0, 2.0]];
        let idx = triangulate(&pts);
        assert_eq!(idx.len(), 12, "n=6 polygon ⇒ (n-2)=4 triangles ⇒ 12 indices");
        // L-shape area: bottom 2×1 + top-left 1×1 = 3.
        assert!((triangle_area_sum(&pts, &idx) - 3.0).abs() < 1e-5);
    }

    fn triangle_area_sum(pts: &[[f32; 2]], idx: &[u16]) -> f32 {
        let mut total = 0.0;
        for tri in idx.chunks(3) {
            let a = pts[tri[0] as usize];
            let b = pts[tri[1] as usize];
            let c = pts[tri[2] as usize];
            total += ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs() * 0.5;
        }
        total
    }
}
