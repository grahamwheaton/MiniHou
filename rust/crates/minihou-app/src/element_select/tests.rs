    use super::*;
    use SelectionMode::{Edges, Faces, Points};

    #[test] fn linked_and_inverse_respect_components_and_visibility_in_all_modes() {
        let m=mesh(vec![0.;24],&[&[0,1,2,3],&[4,5,6,7]]);
        let t=topo(&m);
        for mode in [Points,Edges,Faces] {
            let mut sel=Selection {mode,..Default::default()};
            match mode {Points=>sel.points=vec![0],Edges=>sel.edges=vec![(0,1)],Faces=>sel.tris=vec![0],_=>{}}
            assert!(t.linked_or_inverse(&mut sel,mode,false,|_|true));
            match mode {Points=>assert_eq!(sel.points,vec![0,1,2,3]),Edges=>assert_eq!(sel.edges.len(),4),Faces=>assert_eq!(sel.tris,vec![0,1]),_=>{}}
            assert!(t.linked_or_inverse(&mut sel,mode,true,|_|true));
            match mode {Points=>assert_eq!(sel.points,vec![4,5,6,7]),Edges=>assert!(sel.edges.iter().all(|(a,b)|*a>=4&&*b>=4)),Faces=>assert_eq!(sel.tris,vec![2,3]),_=>{}}
            assert!(t.linked_or_inverse(&mut sel,mode,true,|id|id>=4));
            assert!(sel.is_empty(),"inverse cannot select hidden components");
            assert!(!t.linked_or_inverse(&mut sel,mode,false,|_|true));
        }
    }
    #[test]
    fn brush_merges_preserve_order_detect_noops_and_subtract() {
        let mut selection=Selection {mode:Faces,tris:(0..50_562).collect(),..Default::default()};
        let dab=Selection {mode:Faces,tris:vec![7,2,7,50_562,50_563],..Default::default()};
        let start=std::time::Instant::now();
        assert!(apply_region(&mut selection,dab.clone(),picking::SelectOp::Add));
        assert_eq!(&selection.tris[50_562..],&[50_562,50_563]);
        assert!(!apply_region(&mut selection,dab.clone(),picking::SelectOp::Add));
        assert!(apply_region(&mut selection,dab.clone(),picking::SelectOp::Subtract));
        assert!(!apply_region(&mut selection,dab,picking::SelectOp::Subtract));
        assert!(!selection.tris.contains(&7));
        assert_eq!(&selection.tris[..3],&[0,1,3]);
        eprintln!("Four large brush merges: {:?}",start.elapsed());
        let sparse=Selection {mode:Faces,tris:vec![u32::MAX],..Default::default()};
        assert!(apply_region(&mut selection,sparse.clone(),picking::SelectOp::Add));
        assert!(!apply_region(&mut selection,sparse.clone(),picking::SelectOp::Add));
        assert!(apply_region(&mut selection,sparse,picking::SelectOp::Subtract));
        let subtract=Selection {mode:Points,points:vec![3],..Default::default()};
        assert!(apply_region(&mut selection,subtract,picking::SelectOp::Subtract));
        assert!(selection.is_empty()); assert_eq!(selection.mode,Points);
    }
    #[test]
    fn face_marker_mask_matches_polygon_selection_on_startup_mesh() {
        let graph = minihou_core::Graph::from_json(include_str!("../test_data/demo_graph.json")).unwrap();
        let geo = graph.cook(Some("output1")).unwrap();
        let scene = crate::viewport::Scene::new(&geo);
        let topo = Topology::new(scene.positions(), scene.triangle_indices(), scene.edge_indices());
        let all: Vec<_> = (0..scene.triangles() as u32).collect();
        for selected in [vec![], vec![0, 0, 3, u32::MAX], all] {
            let expected: HashSet<_> = topo.polys_of(&selected).into_iter().collect();
            let start = std::time::Instant::now();
            let mask = topo.polygon_selection_mask(&selected);
            let count = topo.polygons().filter(|&poly| mask[poly as usize]).count();
            eprintln!("Face markers: {} selected triangles, {count} selected polygons, {:?}", selected.len(), start.elapsed());
            assert_eq!(mask.len(), topo.polygons().len());
            for poly in topo.polygons() { assert_eq!(mask[poly as usize], expected.contains(&poly)); }
        }
    }

    /// An `nx` by `ny` grid of quads in z = 0, each quad split by a diagonal; point (i, j) is j * (nx + 1) + i.
    fn grid(nx: u32, ny: u32) -> (Vec<f32>, Vec<u32>, Vec<u32>) {
        let w = nx + 1;
        let p: Vec<f32> = (0..=ny).flat_map(|j| (0..=nx).flat_map(move |i| [i as f32, j as f32, 0.])).collect();
        let (mut t, mut e) = (Vec::new(), Vec::new());
        for j in 0..ny { for i in 0..nx {
            let (a, b, c, d) = (j * w + i, j * w + i + 1, (j + 1) * w + i + 1, (j + 1) * w + i);
            t.extend([a, b, c, a, c, d]);
            e.extend([a, b, b, c, c, d, d, a]);
        } }
        (p, t, e)
    }
    /// Quads given as point lists, fan-triangulated, with their perimeter edges.
    fn mesh(p: Vec<f32>, polys: &[&[u32]]) -> (Vec<f32>, Vec<u32>, Vec<u32>) {
        let (mut t, mut e) = (Vec::new(), Vec::new());
        for f in polys {
            for k in 1..f.len() - 1 { t.extend([f[0], f[k], f[k + 1]]); }
            for k in 0..f.len() { e.extend([f[k], f[(k + 1) % f.len()]]); }
        }
        (p, t, e)
    }
    fn cube() -> (Vec<f32>, Vec<u32>, Vec<u32>) {
        let p = (0..8).flat_map(|c| [(c & 1) as f32, ((c >> 1) & 1) as f32, ((c >> 2) & 1) as f32]).collect();
        mesh(p, &[&[0, 2, 3, 1], &[4, 5, 7, 6], &[0, 1, 5, 4], &[2, 6, 7, 3], &[0, 4, 6, 2], &[1, 3, 7, 5]])
    }
    fn topo((p, t, e): &(Vec<f32>, Vec<u32>, Vec<u32>)) -> Topology { Topology::new(p, t, e) }

    #[test]
    fn parse_group_wildcards_union_exclusion_and_plain_tokens() {
        let t = topo(&grid(3, 2));
        let points = |text| t.parse_group(text, Points).points;
        assert_eq!(points("1"), vec![1]);
        assert_eq!(points("1*"), vec![1, 10, 11]);
        assert_eq!(points("?"), (0..10).collect::<Vec<_>>());
        assert_eq!(points("[0-2]"), vec![0, 1, 2]);
        assert_eq!(points("[013]"), vec![0, 1, 3]);
        assert_eq!(points("1* 2*"), vec![1, 10, 11, 2]);
        assert_eq!(points("* ^1*"), (0..12).filter(|i| !i.to_string().starts_with('1')).collect::<Vec<_>>());
        assert_eq!(points("^1*"), points("* ^1*"));
        assert_eq!(points("^1* 1"), Vec::<u32>::new());

        let edges = t.parse_group("p0-* ^p0-1", Edges).edges;
        assert!(!edges.contains(&(0, 1)));
        assert!(!edges.is_empty());
        assert_eq!(t.parse_group("p1-0", Edges).edges, vec![(1, 0)]);
        assert_eq!(t.parse_group("[0-2]", Faces).tris,
            t.parse_group("0 1 2", Faces).tris);
    }
    fn sorted(mut v: Vec<(u32, u32)>) -> Vec<(u32, u32)> { v.sort(); v }
    fn edge_walk(m: &(Vec<f32>, Vec<u32>, Vec<u32>), a: u32, b: u32, ring: bool) -> Vec<(u32, u32)> {
        let t = topo(m);
        sorted(t.walk(&m.0, t.edge_of(a, b).unwrap(), ring).into_iter().map(|e| t.edges[e as usize]).collect())
    }

    #[test]
    fn polygons_are_rebuilt_from_diagonals() {
        let m = grid(3, 2);
        let t = topo(&m);
        assert_eq!(t.npolys, 6);
        assert_eq!(t.edges.len(), 3 * 3 + 4 * 2);
        assert_eq!(t.poly_of_tri(0), t.poly_of_tri(1));
        assert_ne!(t.poly_of_tri(1), t.poly_of_tri(2));
        assert_eq!(t.poly_tris(t.poly_of_tri(4).unwrap()), &[4, 5]);
        assert!(t.edge_of(0, 5).is_none()); // diagonal of the first quad
    }

    #[test]
    fn face_overlay_uses_one_quad_and_only_its_perimeter() {
        let m = grid(1, 1);
        let topology = topo(&m);
        assert_eq!(topology.polygons().len(), 1);
        assert_eq!(topology.polygon_vertices(0), &[0, 1, 2, 3]);
        assert_eq!(topology.polygon_edges(0).count(), 4);
        assert!(!topology.polygon_edges(0).any(|(a, b)| (a, b) == (0, 3)));
        assert_eq!(topology.polys_of(&[0, 1]), vec![0]);
    }

    #[test]
    fn grid_loop_runs_border_to_border_and_ring_crosses_strip() {
        let m = grid(4, 3); // w = 5
        // interior horizontal edge (1,1)-(2,1): loop is the whole row j = 1
        assert_eq!(edge_walk(&m, 6, 7, false), (0..4).map(|i| (5 + i, 6 + i)).collect::<Vec<_>>());
        // ring of the same edge: every horizontal edge in column i = 1
        assert_eq!(edge_walk(&m, 6, 7, true), (0..4).map(|j| (5 * j + 1, 5 * j + 2)).collect::<Vec<_>>());
        // a vertical interior edge loops the full column
        assert_eq!(edge_walk(&m, 7, 12, false), (0..3).map(|j| (5 * j + 2, 5 * j + 7)).collect::<Vec<_>>());
    }

    #[test]
    fn grid_border_loop_follows_border_and_stops_at_corners() {
        let m = grid(4, 3);
        assert_eq!(edge_walk(&m, 1, 2, false), (0..4).map(|i| (i, i + 1)).collect::<Vec<_>>());
        // a border edge ring runs across the grid through quads only
        assert_eq!(edge_walk(&m, 0, 5, true), (0..5).map(|i| (i, i + 5)).collect::<Vec<_>>());
    }

    #[test]
    fn cube_loop_stops_at_poles_and_ring_wraps() {
        let m = cube();
        assert_eq!(edge_walk(&m, 0, 1, false), vec![(0, 1)]);
        assert_eq!(edge_walk(&m, 0, 1, true), vec![(0, 1), (2, 3), (4, 5), (6, 7)]);
    }

    #[test]
    fn ring_stops_at_triangles_and_ngons() {
        // three quads in a row, the last one replaced by two triangles (a real edge splits it)
        let p: Vec<f32> = (0..2).flat_map(|j| (0..4).flat_map(move |i| [i as f32, j as f32, 0.])).collect();
        let m = mesh(p, &[&[0, 1, 5, 4], &[1, 2, 6, 5], &[2, 3, 7], &[2, 7, 6]]);
        assert_eq!(edge_walk(&m, 0, 4, true), vec![(0, 4), (1, 5), (2, 6)]);
        // the strip as a pentagon at the end: the ring stops at its edge as well
        let p: Vec<f32> = (0..2).flat_map(|j| (0..4).flat_map(move |i| [i as f32, j as f32, 0.])).chain([3.5, 0.5, 0.]).collect();
        let m = mesh(p, &[&[0, 1, 5, 4], &[1, 2, 6, 5], &[2, 3, 8, 7, 6]]);
        assert_eq!(edge_walk(&m, 0, 4, true), vec![(0, 4), (1, 5), (2, 6)]);
    }

    #[test]
    fn loop_stops_at_a_pole_between_quads() {
        // A valence-3 interior vertex (point 4 of a 3-quad fan) cannot be continued: no straight candidate.
        let p = vec![0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0., 0.5, 0.5, 0., -1., 0.5, 0., 0.5, -1., 0.];
        // quads around centre 4: (4,0,...) shapes; edge 5-4 enters the pole
        let m = mesh(p, &[&[4, 3, 5, 0], &[4, 0, 6, 1], &[4, 1, 2, 3]]);
        let t = topo(&m);
        assert_eq!(t.point_edges.get(4).len(), 3);
        assert_eq!(edge_walk(&m, 0, 4, false), vec![(0, 4)]);
    }

    #[test]
    fn ngon_cap_perimeter_is_followed() {
        // a tube with 8 columns: bottom ring 0..8, top ring 8..16, top cap is an 8-gon
        let n = 8u32;
        let p: Vec<f32> = (0..2).flat_map(|j| (0..n).flat_map(move |i| { let a = i as f32 * std::f32::consts::TAU / n as f32; [a.cos(), a.sin(), j as f32] })).collect();
        let sides: Vec<Vec<u32>> = (0..n).map(|i| vec![i, (i + 1) % n, n + (i + 1) % n, n + i]).collect();
        let cap: Vec<u32> = (n..2 * n).collect();
        let mut polys: Vec<&[u32]> = sides.iter().map(|v| v.as_slice()).collect();
        polys.push(&cap);
        let m = mesh(p, &polys);
        assert_eq!(edge_walk(&m, 8, 9, false), sorted((0..n).map(|i| key(n + i, n + (i + 1) % n)).collect()));
        // isolated octagon
        let p: Vec<f32> = (0..8).flat_map(|i| { let a = i as f32 * std::f32::consts::FRAC_PI_4; [a.cos(), a.sin(), 0.] }).collect();
        let m = mesh(p, &[&[0, 1, 2, 3, 4, 5, 6, 7]]);
        assert_eq!(edge_walk(&m, 0, 1, false).len(), 8);
    }

    #[test]
    fn ambiguous_wire_branch_stops() {
        let p = vec![-1., 0., 0., 0., 0., 0., 1., 0.4, 0., 1., -0.4, 0.];
        let t = Topology::new(&p, &[], &[0, 1, 1, 2, 1, 3]);
        assert_eq!(t.walk(&p, t.edge_of(0, 1).unwrap(), false), vec![t.edge_of(0, 1).unwrap()]);
        // a straight wire continues
        let p = vec![0., 0., 0., 1., 0., 0., 2., 0., 0., 3., 0., 0.];
        let t = Topology::new(&p, &[], &[0, 1, 1, 2, 2, 3]);
        assert_eq!(t.walk(&p, t.edge_of(1, 2).unwrap(), false).len(), 3);
    }

    #[test]
    fn loop_selection_in_each_mode() {
        let m = grid(4, 3);
        let t = topo(&m);
        let e = t.edge_of(6, 7).unwrap();
        assert_eq!(t.loop_selection(&m.0, e, false, Points).points.len(), 5);
        // face loop: the column of quads crossed by the ring of a horizontal edge
        let faces = t.loop_selection(&m.0, e, false, Faces);
        assert_eq!(faces.tris.len(), 3 * 2);
        assert_eq!(t.polys_of(&faces.tris).len(), 3);
        assert_eq!(faces, t.loop_selection(&m.0, e, true, Faces));
    }

    #[test]
    fn nearest_edge_ignores_diagonals_and_pick_selects_whole_polygons() {
        let m = grid(1, 1);
        let t = topo(&m);
        // a point on the diagonal near corner 0 is closest to the diagonal but must snap to a real edge
        let e = t.nearest_edge(&m.0, 0, [0.3, 0.25, 0.]).unwrap();
        assert_eq!(t.edges[e as usize], (0, 1));
        let hit = Hit { tri: 0, t: 1., bary: [0.4, 0.3, 0.3] };
        assert_eq!(t.pick(&m.0, &m.1, hit, [0.5, 0.45, 0.], Faces).tris, vec![0, 1]);
        assert_eq!(t.pick(&m.0, &m.1, hit, [0.5, 0.1, 0.], Edges).edges, vec![(0, 1)]);
    }

    #[test]
    fn paths_in_every_mode() {
        let m = grid(4, 3); // w = 5
        let t = topo(&m);
        for method in [Method::Direct, Method::Shortest] {
            // straight along a row
            assert_eq!(t.path(&m.0, Points, 5, 9, method), vec![5, 6, 7, 8, 9]);
            let (a, b) = (t.edge_of(5, 6).unwrap(), t.edge_of(8, 9).unwrap());
            let route = t.path(&m.0, Edges, a, b, method);
            assert_eq!(route.len(), 4);
            assert!(route.iter().all(|&e| { let (x, y) = t.edges[e as usize]; (5..=9).contains(&x) && (5..=9).contains(&y) }));
            let (f0, f3) = (t.poly_of_tri(0).unwrap(), t.poly_of_tri(6).unwrap());
            assert_eq!(t.path(&m.0, Faces, f0, f3, method).len(), 4);
        }
        // diagonal across the grid: direct stays near the chord, shortest has the minimum hop count
        let direct = t.path(&m.0, Points, 0, 18, Method::Direct);
        assert_eq!(direct.len(), 7);
        assert_eq!(t.path(&m.0, Points, 0, 18, Method::Shortest).len(), 7);
        // not the L-shaped route along the border (3 units off the x = y chord)
        let off_chord = direct.iter().map(|&v| { let q = picking::at(&m.0, v); (q[0] - q[1]).abs() }).fold(0f32, f32::max);
        assert!(off_chord <= 2., "{direct:?}");
        // disconnected
        let p = vec![0., 0., 0., 1., 0., 0., 5., 0., 0., 6., 0., 0.];
        let w = Topology::new(&p, &[], &[0, 1, 2, 3]);
        assert!(w.path(&p, Points, 0, 3, Method::Direct).is_empty());
        assert!(w.path(&p, Points, 0, 3, Method::Shortest).is_empty());
    }

    #[test]
    fn direct_path_prefers_a_straight_detailed_chain() {
        // 0 -> 4 straight through 1, 2, 3 or a two-step detour via 5 above the line
        let p = vec![0., 0., 0., 1., 0., 0., 2., 0., 0., 3., 0., 0., 4., 0., 0., 2., 1.5, 0.];
        let t = Topology::new(&p, &[], &[0, 1, 1, 2, 2, 3, 3, 4, 0, 5, 5, 4]);
        assert_eq!(t.path(&p, Points, 0, 4, Method::Direct), vec![0, 1, 2, 3, 4]);
        assert_eq!(t.path(&p, Points, 0, 4, Method::Shortest), vec![0, 5, 4]);
    }

    fn sel(mode: SelectionMode) -> Selection { Selection { mode, ..Default::default() } }

    #[test]
    fn conversion_face_down_and_back_up() {
        let m = cube();
        let t = topo(&m);
        let face = Selection { tris: t.poly_tris(0).to_vec(), ..sel(Faces) };
        let edges = t.convert(&face, Edges);
        assert_eq!(edges.edges, sorted(vec![(0, 2), (2, 3), (1, 3), (0, 1)]));
        let points = t.convert(&face, Points);
        assert_eq!(points.points, vec![0, 1, 2, 3]);
        assert_eq!(t.convert(&edges, Faces).tris, t.poly_tris(0));
        assert_eq!(t.convert(&points, Faces).tris, t.poly_tris(0));
        assert_eq!(t.convert(&edges, Points).points, vec![0, 1, 2, 3]);
        assert_eq!(t.convert(&points, Edges).edges, edges.edges);
        assert_eq!(t.convert(&face, Faces), face);
    }

    #[test]
    fn pick_promote_points_to_face_and_back_is_one_undo_record() {
        let m = cube();
        let t = topo(&m);
        let top = Selection { points: vec![0, 1, 2, 3], ..sel(Points) };
        let faces = t.convert(&top, Faces);
        assert_eq!(faces.tris, t.poly_tris(0));
        assert_eq!(t.convert(&faces, Points).points, vec![0, 1, 2, 3]);
        let mut doc = crate::Document::parse("{\"nodes\":{}}".into()).unwrap();
        let mut history = minihou_app::History::default();
        let before = crate::selection_state(&top, &Selection::default());
        let after = crate::selection_state(&faces, &Selection::default());
        history.record_selection_state(&doc, Some(before), Some(after));
        let crate::HistoryOutcome::Selection(restored) = history.undo_outcome(&mut doc).unwrap() else { panic!("expected selection step") };
        assert_eq!(crate::restore(restored.selection), top);
    }

    #[test]
    fn conversion_partial_selections_do_not_grow() {
        let m = cube();
        let t = topo(&m);
        let one = Selection { points: vec![0], ..sel(Points) };
        assert!(t.convert(&one, Edges).edges.is_empty());
        assert!(t.convert(&one, Faces).tris.is_empty());
        let edge = Selection { edges: vec![(0, 1)], ..sel(Edges) };
        assert!(t.convert(&edge, Faces).tris.is_empty());
        assert_eq!(t.convert(&edge, Points).points, vec![0, 1]);
        // three of a face's four edges: no face
        let three = Selection { edges: vec![(0, 1), (1, 3), (2, 3)], ..sel(Edges) };
        assert!(t.convert(&three, Faces).tris.is_empty());
        // two adjacent faces down to edges share their common edge once
        let two = Selection { tris: [t.poly_tris(0), t.poly_tris(2)].concat(), ..sel(Faces) };
        assert_eq!(t.convert(&two, Edges).edges.len(), 7);
        assert_eq!(t.convert(&two, Points).points.len(), 6);
        // all points select every face; object mode conversions are empty
        let all = Selection { points: (0..8).collect(), ..sel(Points) };
        assert_eq!(t.polys_of(&t.convert(&all, Faces).tris).len(), 6);
        assert_eq!(t.convert(&all, Edges).edges.len(), 12);
        assert!(t.convert(&all, SelectionMode::Object).is_empty());
        assert!(t.convert(&sel(SelectionMode::Object), Points).is_empty());
    }

    #[test]
    fn conversion_quad_diagonal_points_need_the_whole_quad() {
        let m = grid(1, 1);
        let t = topo(&m);
        let diag = Selection { points: vec![0, 1, 2], ..sel(Points) };
        assert!(t.convert(&diag, Faces).tris.is_empty());
        assert_eq!(t.convert(&diag, Edges).edges, vec![(0, 1), (0, 2)]);
    }

    #[test]
    fn click_modifier_mapping() {
        assert_eq!(classify(false, false, false, true), Some(ClickKind::Pick));
        assert_eq!(classify(true, false, false, true), Some(ClickKind::Toggle));
        assert_eq!(classify(false, true, false, true), Some(ClickKind::Path(Method::Direct)));
        assert_eq!(classify(true, true, false, true), Some(ClickKind::Subtract));
        assert_eq!(classify(false, false, true, true), Some(ClickKind::Loop { ring: false, add: false }));
        assert_eq!(classify(true, false, true, true), Some(ClickKind::Loop { ring: false, add: true }));
        assert_eq!(classify(false, true, true, true), Some(ClickKind::Loop { ring: true, add: false }));
        assert_eq!(classify(true, true, true, true), Some(ClickKind::Loop { ring: true, add: true }));
        assert_eq!(classify(false, false, true, false), None);
        assert_eq!(classify(false, true, false, false), Some(ClickKind::Path(Method::Direct)));
    }

    #[test]
    fn clicks_apply_loops_paths_toggles() {
        let m = grid(4, 3);
        let t = topo(&m);
        let pick = |v: u32| Some((Selection { points: vec![v], ..sel(Points) }, 0u32, [0f32; 3]));
        let mut cur = sel(Points);
        // Ctrl-click with nothing selected selects the element
        assert!(t.apply_click(&m.0, ClickKind::Path(Method::Direct), pick(5), Points, &mut cur));
        assert_eq!(cur.points, vec![5]);
        t.apply_click(&m.0, ClickKind::Path(Method::Direct), pick(8), Points, &mut cur);
        assert_eq!(cur.points, vec![5, 6, 7, 8]);
        t.apply_click(&m.0, ClickKind::Path(Method::Direct), pick(18), Points, &mut cur);
        assert_eq!(cur.points.last(), Some(&18));
        assert_eq!(cur.points.len(), 6);
        t.apply_click(&m.0, ClickKind::Toggle, pick(18), Points, &mut cur);
        assert!(!cur.points.contains(&18));
        t.apply_click(&m.0, ClickKind::Subtract, pick(5), Points, &mut cur);
        assert!(!cur.points.contains(&5));
        // Alt-click in edge mode near the bottom edge of quad (1, 1): the row loop; Shift adds the ring
        let mut cur = sel(Edges);
        let tri = (5 * 2) as u32; // first triangle of quad (i = 1, j = 1)
        assert!(t.apply_click(&m.0, ClickKind::Loop { ring: false, add: false }, Some((sel(Edges), tri, [1.5, 1.05, 0.])), Edges, &mut cur));
        assert_eq!(sorted(cur.edges.clone()), (0..4).map(|i| (5 + i, 6 + i)).collect::<Vec<_>>());
        t.apply_click(&m.0, ClickKind::Loop { ring: true, add: true }, Some((sel(Edges), tri, [1.5, 1.05, 0.])), Edges, &mut cur);
        assert_eq!(cur.edges.len(), 4 + 3);
        // misses change nothing for loops, paths and subtraction
        let before = cur.clone();
        assert!(!t.apply_click(&m.0, ClickKind::Loop { ring: false, add: false }, None, Edges, &mut cur));
        assert!(!t.apply_click(&m.0, ClickKind::Path(Method::Direct), None, Edges, &mut cur));
        assert_eq!(cur, before);
        // Shift-click toggles whole polygons
        let mut cur = sel(Faces);
        let face = Some((Selection { tris: vec![0, 1], ..sel(Faces) }, 0, [0.5, 0.5, 0.]));
        t.apply_click(&m.0, ClickKind::Toggle, face.clone(), Faces, &mut cur);
        assert_eq!(cur.tris, vec![0, 1]);
        t.apply_click(&m.0, ClickKind::Toggle, face, Faces, &mut cur);
        assert!(cur.tris.is_empty());
    }

    #[test]
    fn select_more_and_less_change_only_the_selection() {
        let m = grid(4, 4); // w = 5
        let t = topo(&m);
        let mut s = Selection { points: vec![12], ..sel(Points) };
        assert!(t.grow_selection(&mut s, false));
        assert_eq!(s.points[0], 12, "kept elements keep their order");
        assert_eq!(s.points.len(), 9, "face step: the four quads round the point");
        assert!(t.grow_selection(&mut s, true));
        assert_eq!(s.points, vec![12]);
        // faces: whole polygons, one ring by shared points
        let centre = t.poly_of_tri(2 * (4 + 1)).unwrap();
        let mut s = Selection { tris: t.poly_tris(centre).to_vec(), ..sel(Faces) };
        t.grow_selection(&mut s, false);
        assert_eq!(t.polys_of(&s.tris).len(), 9);
        assert_eq!(s.tris.len(), 18);
        let mut s = Selection { edges: vec![(6, 7)], ..sel(Edges) };
        t.grow_selection(&mut s, false);
        assert!(s.edges.contains(&(6, 7)) && s.edges.len() > 4);
        // empty and object selections are left alone
        let mut empty = sel(Points);
        assert!(!t.grow_selection(&mut empty, false));
        assert!(!t.grow_selection(&mut sel(SelectionMode::Object), false));
    }

    #[test]
    fn generic_nodes_take_the_selection_as_group_input() {
        let t = topo(&grid(3, 3));
        let points = Selection { mode: Points, points: vec![1, 2], ..Default::default() };
        let b = t.node_params("blast", &points).unwrap();
        assert_eq!(b, serde_json::json!({"group": "1 2", "domain": "points"}));
        assert_eq!(t.node_params("polybevel", &points).unwrap(), serde_json::json!({"group": "1 2", "group_type": "points"}));
        assert_eq!(t.node_params("transform", &points).unwrap(), serde_json::json!({"group": "1 2"}));
        let edges = Selection { mode: Edges, edges: vec![(0, 1)], ..Default::default() };
        assert_eq!(t.node_params("subdivide", &edges).unwrap(), serde_json::json!({"creases": "p0-1"}));
        assert_eq!(t.node_params("blast", &edges).unwrap()["domain"], "edges");
        assert_eq!(t.node_params("transform", &edges).unwrap()["group"], "0 1");
        for gen in ["box", "grid"] { assert!(t.node_params(gen, &points).is_none(), "{gen}"); assert!(find_input_group_field(gen).is_none()); }
        assert!(t.node_params("blast", &sel(Points)).is_none());
        assert!(t.node_params("blast", &sel(SelectionMode::Object)).is_none());
        assert_eq!(find_input_group_field("blast"), Some(("group", Some("domain"))));
        assert_eq!(find_input_group_field("select_path"), Some(("waypoints", Some("domain"))));
        for node in ["select_loop", "select_ring"] {
            assert_eq!(find_input_group_field(node), Some(("seeds", Some("domain"))));
        }
    }

    #[test]
    fn attribute_create_tab_add_uses_selected_elements_as_context() {
        let t = topo(&grid(4, 3));
        assert_eq!(find_input_group_field("attribute_create"), Some(("group", Some("domain"))));
        let points = Selection { points: vec![2, 5], ..sel(Points) };
        assert_eq!(t.node_params("attribute_create", &points), Some(serde_json::json!({
            "group":"2 5", "domain":"points", "group_type":"points"})));
        let edges = Selection { edges: vec![(6, 7), (7, 8)], ..sel(Edges) };
        assert_eq!(t.node_params("attribute_create", &edges), Some(serde_json::json!({
            "group":"6 7 8", "domain":"points", "group_type":"points"})));
        let faces = Selection { tris: vec![0, 1, 2, 3], ..sel(Faces) };
        assert_eq!(t.node_params("attribute_create", &faces), Some(serde_json::json!({
            "group":"0 1", "domain":"primitives", "group_type":"primitives"})));
        assert!(t.node_params("attribute_create", &sel(Points)).is_none());
        assert!(t.node_params("attribute_create", &sel(SelectionMode::Object)).is_none());
    }

    fn pick_group_create_tags_the_selection_with_a_free_name() {
        let m = grid(4, 3);
        let t = topo(&m);
        let faces = Selection { tris: vec![0, 1, 2, 3], ..sel(Faces) };
        assert_eq!(t.node_params("group_create", &faces).unwrap(),
            serde_json::json!({"domain": "primitives", "method": "pattern", "pattern": "0 1"}));
        let points = Selection { points: vec![2, 5], ..sel(Points) };
        assert_eq!(t.node_params("group_create", &points).unwrap()["pattern"], "2 5");
        let edges = Selection { edges: vec![(6, 7)], ..sel(Edges) };
        assert_eq!(t.node_params("group_create", &edges).unwrap(),
            serde_json::json!({"domain": "edges", "method": "pattern", "pattern": "6-7"}));
        assert!(t.node_params("group_create", &sel(Faces)).is_none(), "an empty selection creates nothing");
        let mut doc = serde_json::json!({"nodes": {"a": {"type": "box"}}});
        assert_eq!(free_group_name(&doc), "group1");
        doc["nodes"]["g"] = serde_json::json!({"type": "group_create", "params": {"name": "group1"}});
        assert_eq!(free_group_name(&doc), "group2");
        doc["nodes"]["h"] = serde_json::json!({"type": "group_create", "params": {"name": "group3"}});
        assert_eq!(free_group_name(&doc), "group2");
        assert_eq!(free_group_name_with_prefix(&doc, "boundary"), "boundary1");
        doc["nodes"]["b"] = serde_json::json!({"type": "boundary_select", "params": {"output_group_name": "boundary1"}});
        assert_eq!(free_group_name_with_prefix(&doc, "boundary"), "boundary2");
        assert_eq!(free_group_name_with_prefix(&doc, "sharp_edges"), "sharp_edges1");
        doc["nodes"]["s"] = serde_json::json!({"type": "select_sharp_edges", "params": {"output_group_name": "sharp_edges1"}});
        assert_eq!(free_group_name_with_prefix(&doc, "sharp_edges"), "sharp_edges2");
    }

    #[test]
    fn pick_selection_nodes_take_their_seeds_from_the_selection() {
        let m = grid(4, 3);
        let t = topo(&m);
        let points = Selection { points: vec![7, 3, 9], ..sel(Points) };
        assert_eq!(t.node_params("grow", &points).unwrap(), serde_json::json!({"group": "7 3 9", "domain": "points"}));
        assert_eq!(t.node_params("select_path", &points).unwrap(), serde_json::json!({"waypoints": "7 3 9", "domain": "points"}));
        let edges = Selection { edges: vec![(6, 7), (7, 8)], ..sel(Edges) };
        assert_eq!(t.node_params("select_path", &edges).unwrap(), serde_json::json!({"waypoints": "p6-7 p7-8", "domain": "edges"}));
        assert_eq!(t.node_params("select_loop", &edges).unwrap(), serde_json::json!({"seeds": "p6-7 p7-8", "domain": "edges"}));
        assert_eq!(t.node_params("select_ring", &edges).unwrap(), serde_json::json!({"seeds": "p6-7 p7-8", "domain": "edges"}));
        let face = Selection { tris: vec![0, 1], ..sel(Faces) };
        assert_eq!(t.node_params("select_path", &face).unwrap(), serde_json::json!({"waypoints": "0", "domain": "primitives"}));
        assert_eq!(t.node_params("grow", &face).unwrap(), serde_json::json!({"group": "0", "domain": "primitives"}));
        // faces convert to their edges as loop seeds
        for node in ["select_loop", "select_ring"] {
            let params = t.node_params(node, &face).unwrap();
            assert_eq!(params["domain"], "primitives");
            assert_eq!(params["seeds"].as_str().unwrap().split(' ').count(), 4);
            let adjacent_points = Selection { points: vec![6, 7], ..sel(Points) };
            assert_eq!(t.node_params(node, &adjacent_points).unwrap()["domain"], "points");
        }
        assert_eq!(t.node_params("blast", &points).unwrap(), serde_json::json!({"domain": "points", "group": "7 3 9"}));
        assert_eq!(t.node_params("split", &points), Some(serde_json::json!({"group_type":"points","group":"7 3 9"})));
        assert_eq!(t.node_params("blast", &edges).unwrap(), serde_json::json!({"domain": "edges", "group": "6-7 7-8"}));
        assert_eq!(t.node_params("blast", &face).unwrap(), serde_json::json!({"domain": "primitives", "group": "0"}));
        assert!(t.node_params("box", &points).is_none());
        // Tab > Edge Slide and GG take the selected edges as the node's group
        assert_eq!(t.node_params("edge_slide", &edges).unwrap(), serde_json::json!({"group": "p6-7 p7-8"}));
        assert_eq!(t.node_params("boundary_select", &edges).unwrap(), serde_json::json!({"group": "p6-7 p7-8", "domain": "edges"}));
        assert_eq!(t.node_params("select_sharp_edges", &edges).unwrap(), serde_json::json!({"group": "p6-7 p7-8"}));
        assert!(t.node_params("edge_slide", &sel(Edges)).is_none());
        assert!(t.node_params("grow", &sel(Points)).is_none());
    }

    #[test]
    fn subdivide_crease_group_tokens_round_trip() {
        let mesh = grid(4, 3);
        let topo = topo(&mesh);
        let edges = Selection { edges: vec![(6, 7), (7, 8)], ..sel(Edges) };
        let text = topo.group_string(&edges);
        assert_eq!(text, "p6-7 p7-8");
        let decoded: Vec<(u32,u32)> = text.split_whitespace().map(|token| {
            let (a,b) = token.strip_prefix('p').unwrap().split_once('-').unwrap();
            (a.parse().unwrap(), b.parse().unwrap())
        }).collect();
        assert_eq!(decoded, edges.edges);
    }

    #[test]
    fn complete_expands_polygons_and_drops_diagonals() {
        let m = grid(2, 1);
        let t = topo(&m);
        let s = t.complete(Selection { tris: vec![1], edges: vec![(0, 4), (0, 1)], ..sel(Faces) });
        assert_eq!(s.tris, vec![0, 1]);
        assert_eq!(s.edges, vec![(0, 1)]);
    }

    /// Polygon edges of ngons and a beveled box corner never include fan/triangulation diagonals, and the
    /// rebuilt polygons own exactly their sides.
    #[test]
    fn ngon_and_bevel_corner_edges_have_no_interior_diagonals() {
        use minihou_core::Geometry;
        let check = |g: &Geometry, what: &str| {
            let scene = crate::viewport::Scene::new(g);
            let t = &g.topology;
            let mut sides = HashSet::new();
            for f in 0..g.nprims() {
                let vs: Vec<u32> = (t.prim_offsets[f] as usize..t.prim_offsets[f + 1] as usize).map(|v| t.vertex_point[v] as u32).collect();
                for i in 0..vs.len() { sides.insert(key(vs[i], vs[(i + 1) % vs.len()])); }
            }
            let edges: HashSet<(u32, u32)> = scene.edge_indices().chunks_exact(2).map(|e| key(e[0], e[1])).collect();
            assert_eq!(edges, sides, "{what}: edge list must be exactly the polygon sides");
            let topo = Topology::new(scene.positions(), scene.triangle_indices(), scene.edge_indices());
            assert_eq!(topo.polygons().len(), g.nprims(), "{what}: one polygon per primitive");
            for poly in topo.polygons() {
                let n = topo.polygon_vertices(poly).len();
                assert_eq!(topo.polygon_edges(poly).count(), n, "{what}: polygon {poly} outline must be its {n} sides");
                assert!(topo.polygon_edges(poly).all(|(a, b)| sides.contains(&key(a, b))));
            }
        };
        // pentagon, hexagon and a concave (arrow) hexagon side by side
        let mut p = Vec::new(); let mut vp = Vec::new(); let mut offs = vec![0];
        for (k, n) in [5usize, 6].into_iter().enumerate() {
            let base = p.len() as i32 / 3;
            for i in 0..n { let a = i as f32 / n as f32 * std::f32::consts::TAU; p.extend([a.cos() + k as f32 * 3., a.sin(), 0.]); vp.push(base + i as i32); }
            offs.push(vp.len() as i32);
        }
        let base = p.len() as i32 / 3;
        for q in [[0., 0.], [2., 0.], [2., 2.], [1., 0.8], [0., 2.], [-0.5, 1.]] { p.extend([q[0] + 9., q[1], 0.]); }
        vp.extend((0..6).map(|i| base + i)); offs.push(vp.len() as i32);
        check(&Geometry::from_arrays(p, vp, offs, None).unwrap(), "ngons");
        // the beveled box corner (three beveled edges meeting, divisions 2-4: hexagon to octagon caps and corner patches)
        for div in [2, 3, 4] {
            let doc = minihou_app::Document::parse(serde_json::json!({"nodes":{"box":{"type":"box"},
                "bev":{"type":"polybevel","inputs":{"geo":"box"},"params":{"distance":0.1,"divisions":div,"edges":"0-1 0-2 0-4 0-3 0-5 0-6 0-7"}}},
                "display":"bev"}).to_string()).unwrap();
            let g = doc.session().unwrap().cook(Some("bev")).unwrap();
            check(&g, &format!("bevel divisions {div}"));
        }
    }

    #[test]
    fn selections_follow_their_topology() {
        let sel = |a: u32, b: u32| Selection { mode: SelectionMode::Edges, edges: vec![(a, b)], ..Default::default() };
        let mut memory = SelectionMemory::default();
        let boxed = topology_key(8, &[0, 1, 2], &[0, 1, 1, 2]);
        let beveled = topology_key(12, &[0, 1, 2, 2, 3, 0], &[0, 1, 1, 2, 2, 3]);
        assert_ne!(boxed, beveled);
        assert_eq!(memory.retarget(boxed, sel(0, 1)), sel(0, 1));
        // the same topology (a param drag) keeps the selection
        assert_eq!(memory.retarget(boxed, sel(0, 1)), sel(0, 1));
        // bevel changes the numbering: the old point pairs must not be highlighted on the new mesh
        assert!(memory.retarget(beveled, sel(0, 1)).is_empty());
        // cancel / undo back to the box restores its selection; the beveled one is kept too
        assert_eq!(memory.retarget(boxed, sel(5, 6)), sel(0, 1));
        assert_eq!(memory.retarget(beveled, Selection::default()), sel(5, 6));
    }
