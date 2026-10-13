//! `ThemeDocument`, the `rastai.theme/1` row of contracts 5.2 and 5.6 (owner decision N1; layout 4.2a): the
//! built-in light and dark themes of `@rastai/ui` and, from M8, theme plugins.
//!
//! Every color role of contracts 5.6 is its own optional field of `ThemeColors`, never a key of a free map. So a typo,
//! an unknown role or a locked security-UI name (`--rr-locked-egress`) is an unknown field, and layer 2 refuses it
//! with `RRS-E1003`: no theme can set a locked token (owner decision N7). A role left out comes from the theme's
//! `base`. The other token groups are the full row and every field in them is required.
//!
//! Layer 3 (validate/meaning/theme.rs) checks that each color is `#rrggbb`. The token group is `typography`, never
//! `type`: `type` is the shared header's field (contracts 5.6).

use serde::{Deserialize, Serialize};

use crate::describe::types::{
    FieldDescription, FieldKind, OPTIONAL, REQUIRED, TypeDescription, enum_of, field, list_of, type_description,
};
use crate::header::{SharedHeader, header_fields};

/// The header's `type` value.
pub const THEME_TYPE_NAME: &str = "rastai.theme";
pub(crate) const THEME_SCHEMA_VERSION: u32 = 1;
const THEME_SCHEMA_ID: &str = "rastai.theme/1";
/// Theme files are found by this ending (`<plugin>/themes/<name>.theme.json`).
pub(crate) const THEME_FILE_EXTENSION: &str = ".theme.json";

// The described type names, which `FieldKind::Type` refers to.
pub(crate) const THEME_DOCUMENT_NAME: &str = "ThemeDocument";
const COLORS_NAME: &str = "ThemeColors";
const TYPOGRAPHY_NAME: &str = "ThemeTypography";
const FONT_WEIGHTS_NAME: &str = "ThemeFontWeights";
const FONT_SIZES_NAME: &str = "ThemeFontSizes";
const LINE_HEIGHTS_NAME: &str = "ThemeLineHeights";
const DENSITY_NAME: &str = "ThemeDensity";
const SPACING_NAME: &str = "ThemeSpacing";
const RADIUS_NAME: &str = "ThemeRadius";
const MOTION_NAME: &str = "ThemeMotion";
const FOCUS_RING_NAME: &str = "ThemeFocusRing";
const ELEVATION_NAME: &str = "ThemeElevation";

/// The values of `base`, as serde writes `ThemeBase`.
const THEME_BASE_VALUES: &[&str] = &["light", "dark"];

/// Every color role key of contracts 5.6, in the table's order, with what it colors. `ThemeColors` has one field
/// for each, in this order; tests/describe_matches_struct.rs checks the two agree.
pub const COLOR_ROLES: &[(&str, &str)] = &[
    ("background", "Window background."),
    ("surface", "Panels and cards."),
    ("surface_raised", "Menus, popovers and raised cards."),
    ("surface_sunken", "Wells and inputs set into a surface."),
    ("overlay", "The scrim behind dialogs."),
    ("text", "Body text."),
    ("text_muted", "Secondary text."),
    ("text_disabled", "Text of disabled controls."),
    ("text_on_accent", "Text on an accent fill."),
    ("border", "Control and panel borders."),
    ("border_strong", "Borders that must stand out."),
    ("divider", "Lines between list rows and sections."),
    ("accent", "Primary actions and the selected item."),
    ("accent_hover", "An accent control under the pointer."),
    ("accent_active", "An accent control being pressed."),
    ("selection", "Selected text and rows."),
    ("focus_ring", "The keyboard focus ring."),
    ("success", "Success states."),
    ("series_1", "Chart series 1."),
    ("series_2", "Chart series 2."),
    ("series_3", "Chart series 3."),
    ("series_4", "Chart series 4."),
    ("series_5", "Chart series 5."),
    ("series_6", "Chart series 6."),
    ("series_7", "Chart series 7."),
    ("series_8", "Chart series 8."),
    ("sequential_low", "Low end of sequential color maps."),
    ("sequential_high", "High end of sequential color maps."),
    ("diverging_negative", "Negative end of diverging color maps."),
    ("diverging_zero", "Middle of diverging color maps."),
    ("diverging_positive", "Positive end of diverging color maps."),
    ("canvas_background", "Designer canvas background."),
    ("canvas_grid", "Designer canvas grid lines."),
    ("canvas_node", "Graph node fill."),
    ("canvas_node_border", "Graph node border."),
    ("canvas_node_selected", "Selected graph node."),
    ("canvas_edge", "Graph edges."),
    ("canvas_edge_active", "Edges carrying data in a live view."),
    ("canvas_label", "Labels on the canvas."),
    ("canvas_port", "Node input and output ports."),
    ("confidence_tint", "Chat text shading by model confidence."),
];

/// One `rastai.theme/1` file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThemeDocument {
    #[serde(flatten)]
    pub header: SharedHeader,
    /// Shown in the theme picker.
    pub name: String,
    /// Light or dark: roles left out of `colors`, and the locked security-UI set, come from it.
    pub base: ThemeBase,
    pub colors: ThemeColors,
    pub typography: ThemeTypography,
    pub density: ThemeDensity,
    pub radius_px: ThemeRadius,
    pub motion: ThemeMotion,
    pub focus_ring: ThemeFocusRing,
    pub elevation: ThemeElevation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeBase {
    Light,
    Dark,
}

/// One optional `#rrggbb` per color role (`COLOR_ROLES`). A role left out is written nowhere, so a file read and
/// written back sets the same roles.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeColors {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface_raised: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface_sunken: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overlay: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_muted: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_disabled: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_on_accent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border_strong: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub divider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent_hover: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent_active: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus_ring: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_1: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_2: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_3: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_4: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_5: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_6: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_7: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_8: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequential_low: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequential_high: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diverging_negative: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diverging_zero: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diverging_positive: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canvas_background: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canvas_grid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canvas_node: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canvas_node_border: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canvas_node_selected: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canvas_edge: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canvas_edge_active: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canvas_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canvas_port: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence_tint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThemeTypography {
    /// A CSS font-family list for the interface.
    pub font_family_ui: String,
    /// A CSS font-family list for code and numbers.
    pub font_family_mono: String,
    pub font_weights: ThemeFontWeights,
    pub font_sizes_px: ThemeFontSizes,
    pub line_heights: ThemeLineHeights,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeFontWeights {
    pub regular: u32,
    pub medium: u32,
    pub bold: u32,
}

/// Font sizes in CSS pixels. `xxl`, the display size for headings, may be left out: then it is `xl` (contracts 5.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeFontSizes {
    pub xs: u32,
    pub sm: u32,
    pub md: u32,
    pub lg: u32,
    pub xl: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xxl: Option<u32>,
}

/// Unitless CSS line heights (1.5 is one and a half times the font size).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThemeLineHeights {
    pub body: f64,
    pub heading: f64,
    pub code: f64,
}

/// The two spacing scales; `ui.density` picks one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeDensity {
    pub comfortable: ThemeSpacing,
    pub compact: ThemeSpacing,
}

/// One spacing scale: `--rr-space-<n>` is `spacing_px[n]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeSpacing {
    pub spacing_px: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeRadius {
    pub small: u32,
    pub medium: u32,
    pub large: u32,
}

/// `ui.reduced_motion` sets both durations to 0 (contracts 5.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeMotion {
    pub fast_ms: u32,
    pub slow_ms: u32,
    /// A CSS easing function, such as `cubic-bezier(0.2, 0, 0, 1)`.
    pub easing: String,
}

/// The ring's color is the `focus_ring` role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeFocusRing {
    pub width_px: u32,
    pub offset_px: u32,
}

/// Three CSS box-shadow values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeElevation {
    pub level_1: String,
    pub level_2: String,
    pub level_3: String,
}

/// The color roles that are set, with their role key, in `COLOR_ROLES` order. Layer 3 checks each one.
pub fn set_colors(colors: &ThemeColors) -> Vec<(&'static str, &str)> {
    let values = [
        &colors.background,
        &colors.surface,
        &colors.surface_raised,
        &colors.surface_sunken,
        &colors.overlay,
        &colors.text,
        &colors.text_muted,
        &colors.text_disabled,
        &colors.text_on_accent,
        &colors.border,
        &colors.border_strong,
        &colors.divider,
        &colors.accent,
        &colors.accent_hover,
        &colors.accent_active,
        &colors.selection,
        &colors.focus_ring,
        &colors.success,
        &colors.series_1,
        &colors.series_2,
        &colors.series_3,
        &colors.series_4,
        &colors.series_5,
        &colors.series_6,
        &colors.series_7,
        &colors.series_8,
        &colors.sequential_low,
        &colors.sequential_high,
        &colors.diverging_negative,
        &colors.diverging_zero,
        &colors.diverging_positive,
        &colors.canvas_background,
        &colors.canvas_grid,
        &colors.canvas_node,
        &colors.canvas_node_border,
        &colors.canvas_node_selected,
        &colors.canvas_edge,
        &colors.canvas_edge_active,
        &colors.canvas_label,
        &colors.canvas_port,
        &colors.confidence_tint,
    ];
    let mut set = Vec::new();
    for ((role, _), value) in COLOR_ROLES.iter().zip(values) {
        if let Some(color) = value {
            set.push((*role, color.as_str()));
        }
    }
    set
}

/// The theme document and every part of it, for the registry (describe/registry.rs).
pub fn describe_theme_types() -> Vec<TypeDescription> {
    vec![
        describe_theme_document(),
        describe_theme_colors(),
        describe_theme_typography(),
        type_description(
            FONT_WEIGHTS_NAME,
            THEME_SCHEMA_ID,
            "Font weights, 100 to 900.",
            integer_fields(&["regular", "medium", "bold"], "CSS font weight."),
        ),
        describe_theme_font_sizes(),
        type_description(
            LINE_HEIGHTS_NAME,
            THEME_SCHEMA_ID,
            "Unitless CSS line heights.",
            number_fields(
                &["body", "heading", "code"],
                "Line height as a multiple of the font size.",
            ),
        ),
        describe_theme_density(),
        describe_theme_spacing(),
        type_description(
            RADIUS_NAME,
            THEME_SCHEMA_ID,
            "Corner radii in CSS pixels.",
            integer_fields(&["small", "medium", "large"], "Corner radius in CSS pixels."),
        ),
        describe_theme_motion(),
        describe_theme_focus_ring(),
        type_description(
            ELEVATION_NAME,
            THEME_SCHEMA_ID,
            "Three shadow levels.",
            text_fields(&["level_1", "level_2", "level_3"], "A CSS box-shadow value."),
        ),
    ]
}

fn describe_theme_document() -> TypeDescription {
    let mut fields = header_fields();
    fields.extend([
        field("name", FieldKind::String, REQUIRED, "Shown in the theme picker."),
        field(
            "base",
            enum_of(THEME_BASE_VALUES),
            REQUIRED,
            "Light or dark: missing roles and the locked set come from it.",
        ),
        type_field("colors", COLORS_NAME, "One optional #rrggbb per color role."),
        type_field(
            "typography",
            TYPOGRAPHY_NAME,
            "Font families, weights, sizes and line heights.",
        ),
        type_field("density", DENSITY_NAME, "The comfortable and compact spacing scales."),
        type_field("radius_px", RADIUS_NAME, "Corner radii."),
        type_field("motion", MOTION_NAME, "Animation durations and easing."),
        type_field("focus_ring", FOCUS_RING_NAME, "Focus ring width and offset."),
        type_field("elevation", ELEVATION_NAME, "Shadow levels."),
    ]);
    TypeDescription {
        name: THEME_DOCUMENT_NAME.to_string(),
        schema_id: THEME_SCHEMA_ID.to_string(),
        doc: "A theme: color roles and the other design tokens (contracts 5.6).".to_string(),
        fields,
    }
}

fn describe_theme_colors() -> TypeDescription {
    let fields = COLOR_ROLES
        .iter()
        .map(|(role, doc)| field(role, FieldKind::String, OPTIONAL, &format!("{doc} A #rrggbb color.")))
        .collect();
    type_description(
        COLORS_NAME,
        THEME_SCHEMA_ID,
        "Color roles; a role left out comes from the base theme. Locked --rr-locked-* tokens are not roles.",
        fields,
    )
}

fn describe_theme_font_sizes() -> TypeDescription {
    let mut fields = integer_fields(&["xs", "sm", "md", "lg", "xl"], "Font size in CSS pixels.");
    fields.push(field(
        "xxl",
        FieldKind::Integer,
        OPTIONAL,
        "Display size for headings, in CSS pixels. Left out, it is xl.",
    ));
    type_description(
        FONT_SIZES_NAME,
        THEME_SCHEMA_ID,
        "Font sizes in CSS pixels, smallest first.",
        fields,
    )
}

fn describe_theme_typography() -> TypeDescription {
    type_description(
        TYPOGRAPHY_NAME,
        THEME_SCHEMA_ID,
        "Typography tokens.",
        vec![
            field(
                "font_family_ui",
                FieldKind::String,
                REQUIRED,
                "CSS font-family list for the interface.",
            ),
            field(
                "font_family_mono",
                FieldKind::String,
                REQUIRED,
                "CSS font-family list for code.",
            ),
            type_field("font_weights", FONT_WEIGHTS_NAME, "Regular, medium and bold weights."),
            type_field(
                "font_sizes_px",
                FONT_SIZES_NAME,
                "Font sizes xs to xl, and the optional xxl display size for headings (xl when left out).",
            ),
            type_field(
                "line_heights",
                LINE_HEIGHTS_NAME,
                "Body, heading and code line heights.",
            ),
        ],
    )
}

fn describe_theme_density() -> TypeDescription {
    type_description(
        DENSITY_NAME,
        THEME_SCHEMA_ID,
        "Spacing scales; ui.density picks one.",
        vec![
            type_field("comfortable", SPACING_NAME, "The default scale."),
            type_field("compact", SPACING_NAME, "The tighter scale."),
        ],
    )
}

fn describe_theme_motion() -> TypeDescription {
    type_description(
        MOTION_NAME,
        THEME_SCHEMA_ID,
        "Animation timing; ui.reduced_motion sets both durations to 0.",
        vec![
            field(
                "fast_ms",
                FieldKind::Integer,
                REQUIRED,
                "Short transitions, in milliseconds.",
            ),
            field(
                "slow_ms",
                FieldKind::Integer,
                REQUIRED,
                "Long transitions, in milliseconds.",
            ),
            field("easing", FieldKind::String, REQUIRED, "A CSS easing function."),
        ],
    )
}

fn describe_theme_spacing() -> TypeDescription {
    type_description(
        SPACING_NAME,
        THEME_SCHEMA_ID,
        "One spacing scale.",
        vec![field(
            "spacing_px",
            list_of(FieldKind::Integer),
            REQUIRED,
            "Spacing steps in CSS pixels; --rr-space-<n> is step n.",
        )],
    )
}

fn describe_theme_focus_ring() -> TypeDescription {
    type_description(
        FOCUS_RING_NAME,
        THEME_SCHEMA_ID,
        "Focus ring size; its color is the focus_ring role.",
        vec![
            field("width_px", FieldKind::Integer, REQUIRED, "Ring width in CSS pixels."),
            field(
                "offset_px",
                FieldKind::Integer,
                REQUIRED,
                "Gap between the ring and the control.",
            ),
        ],
    )
}

fn type_field(name: &str, type_name: &str, doc: &str) -> FieldDescription {
    field(name, FieldKind::Type(type_name.to_string()), REQUIRED, doc)
}

fn integer_fields(names: &[&str], doc: &str) -> Vec<FieldDescription> {
    names
        .iter()
        .map(|name| field(name, FieldKind::Integer, REQUIRED, doc))
        .collect()
}

fn number_fields(names: &[&str], doc: &str) -> Vec<FieldDescription> {
    names
        .iter()
        .map(|name| field(name, FieldKind::Number, REQUIRED, doc))
        .collect()
}

fn text_fields(names: &[&str], doc: &str) -> Vec<FieldDescription> {
    names
        .iter()
        .map(|name| field(name, FieldKind::String, REQUIRED, doc))
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn base_values_are_what_serde_writes() {
        let written: Vec<String> = [ThemeBase::Light, ThemeBase::Dark]
            .iter()
            .map(|base| serde_json::to_value(base).unwrap().as_str().unwrap().to_string())
            .collect();
        assert_eq!(written, THEME_BASE_VALUES);
    }

    #[test]
    fn set_colors_lists_only_the_roles_that_are_set_in_role_order() {
        let colors = ThemeColors {
            canvas_port: Some("#000000".to_string()),
            background: Some("#ffffff".to_string()),
            ..ThemeColors::default()
        };
        assert_eq!(
            set_colors(&colors),
            vec![("background", "#ffffff"), ("canvas_port", "#000000")]
        );
        assert!(set_colors(&ThemeColors::default()).is_empty());
    }

    #[test]
    fn there_are_forty_one_roles_and_no_locked_or_type_names() {
        assert_eq!(COLOR_ROLES.len(), 41);
        for (role, _) in COLOR_ROLES {
            assert!(!role.starts_with("--") && !role.contains("locked"), "{role}");
        }
        let names: Vec<String> = describe_theme_document()
            .fields
            .iter()
            .map(|item| item.name.clone())
            .collect();
        assert!(names.contains(&"typography".to_string()));
        // `type` is the header's field only; no token group takes the name.
        assert_eq!(names.iter().filter(|name| *name == "type").count(), 1);
    }
}
