//! Source-derived native cover history and separate incomplete top-stock diagnostic.
#[allow(dead_code)]
#[path = "native_plate_support/host.rs"]
mod host;
#[path = "prusa_native_cover_support/mod.rs"]
mod model;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_top_contour_stock_is_native_but_incomplete() {
    model::source_top_contour_stock_is_native_but_incomplete().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_derived_cover_native_history() {
    model::source_derived_cover_native_history().await;
}

mod mesh_topology {
    #[cfg(test)]
    mod tests {
        use crate::model::mesh_topology::component_count;

        #[test]
        fn closed_tetrahedron_has_one_edge_connected_component() {
            let tetra = [[0, 2, 1], [0, 1, 3], [1, 2, 3], [2, 0, 3]];
            assert_eq!(component_count(&tetra), 1);
        }

        #[test]
        fn closed_tetrahedra_touching_only_at_vertex_remain_separate() {
            let tetrahedra = [
                [0, 2, 1],
                [0, 1, 3],
                [1, 2, 3],
                [2, 0, 3],
                [0, 5, 4],
                [0, 4, 6],
                [4, 5, 6],
                [5, 0, 6],
            ];
            assert_eq!(component_count(&tetrahedra), 2);
        }
    }
}
