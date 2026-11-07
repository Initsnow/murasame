use mocari::moc3::Moc3DrawableMesh;

pub fn drawable_contains_point(mesh: &Moc3DrawableMesh, x: f32, y: f32) -> bool {
    let Some(first) = mesh.vertices().first() else {
        return false;
    };

    let [first_x, first_y] = first.position();
    let mut min_x = first_x;
    let mut min_y = first_y;
    let mut max_x = first_x;
    let mut max_y = first_y;

    for vertex in mesh.vertices().iter().skip(1) {
        let [vertex_x, vertex_y] = vertex.position();
        min_x = min_x.min(vertex_x);
        min_y = min_y.min(vertex_y);
        max_x = max_x.max(vertex_x);
        max_y = max_y.max(vertex_y);
    }

    (min_x..=max_x).contains(&x) && (min_y..=max_y).contains(&y)
}
