//! Geometry knobs shared by text, shape and layout code.

/// Layout and sizing parameters.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Metrics {
    pub font_size: f32,
    pub line_height: f32,
    pub max_label_width: f32,
    pub pad_x: f32,
    pub pad_y: f32,
    pub radius: f32,
    pub rank_gap: f32,
    pub node_gap: f32,
    pub edge_label_font: f32,
    pub cluster_pad: f32,
    pub cluster_title: f32,
    pub width_step: f32,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            font_size: 14.0,
            line_height: 20.0,
            max_label_width: 220.0,
            pad_x: 18.0,
            pad_y: 12.0,
            radius: 12.0,
            rank_gap: 64.0,
            node_gap: 40.0,
            edge_label_font: 13.0,
            cluster_pad: 24.0,
            cluster_title: 28.0,
            width_step: 24.0,
        }
    }
}
