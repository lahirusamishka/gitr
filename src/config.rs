use egui::Color32;

pub const LANE_COLORS: [Color32; 6] = [
    Color32::from_rgb(0x89, 0xb4, 0xfa), // blue
    Color32::from_rgb(0xf3, 0x8b, 0xa8), // red/pink
    Color32::from_rgb(0xa6, 0xe3, 0xa1), // green
    Color32::from_rgb(0xcb, 0xa6, 0xf7), // mauve
    Color32::from_rgb(0xf9, 0xe2, 0xaf), // yellow
    Color32::from_rgb(0x89, 0xdc, 0xeb), // sky
];

pub const ROW_HEIGHT: f32 = 24.0;
pub const LANE_WIDTH: f32 = 14.0;
pub const NODE_RADIUS: f32 = 5.0;
pub const LINE_WIDTH: f32 = 2.0;
pub const GRAPH_PAD_LEFT: f32 = 12.0;
pub const GRAPH_PAD_RIGHT: f32 = 16.0;

pub const COL_AUTHOR: f32 = 130.0;
pub const COL_DATE: f32 = 140.0;
pub const COL_HASH: f32 = 85.0;

pub const C_TEXT: Color32 = Color32::from_rgb(0xcd, 0xd6, 0xf4);
pub const C_SUBTEXT: Color32 = Color32::from_rgb(0x93, 0x99, 0xb2);
pub const C_HASH: Color32 = Color32::from_rgb(0xf9, 0xe2, 0xaf);
pub const C_SEL: Color32 = Color32::from_rgba_premultiplied(0x31, 0x32, 0x44, 200);
pub const C_HOVER: Color32 = Color32::from_rgba_premultiplied(0x28, 0x28, 0x38, 120);

pub fn lane_color(lane: usize) -> Color32 {
    LANE_COLORS[lane % LANE_COLORS.len()]
}
