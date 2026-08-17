mod layout;
mod render;

#[allow(unused_imports)]
pub(in crate::app) use layout::{
    DisplayRowLayout, app_areas, build_display_layout, hit_test_display_rows, hit_test_todo_text,
    maximum_viewport_start, reconcile_viewport_start, todo_viewport_area,
};
pub(in crate::app) use render::{render_branch_sections, render_footer, render_status_bar};
