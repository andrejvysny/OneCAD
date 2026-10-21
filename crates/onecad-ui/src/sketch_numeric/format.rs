//! Display-only rounding; geometry keeps full precision.

pub(crate) fn format_measure(value: f64) -> String {
    if !value.is_finite() {
        return "0".into();
    }
    let text = format!("{:.2}", (value * 100.0).round() / 100.0);
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() || text == "-0" || text == "-" {
        "0".into()
    } else {
        text.into()
    }
}

/// Sweeps and signed angles read as a positive turn.
pub(crate) fn format_angle_degrees(deg: f64) -> String {
    if !deg.is_finite() {
        return "0".into();
    }
    let wrapped = deg.rem_euclid(360.0);
    // rem_euclid can land on 360.0 for tiny negatives; format_measure then reads "360".
    format_measure(if wrapped >= 359.995 { 0.0 } else { wrapped })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_round_to_two_decimals_and_trim() {
        assert_eq!(format_measure(1.7949439391), "1.79");
        assert_eq!(format_measure(5.0), "5");
        assert_eq!(format_measure(9.999999999999998), "10");
        assert_eq!(format_measure(2.5), "2.5");
        assert_eq!(format_measure(-0.001), "0");
        assert_eq!(format_measure(-1.256), "-1.26");
        assert_eq!(format_measure(100.0), "100");
        assert_eq!(format_measure(f64::NAN), "0");
        assert_eq!(format_measure(f64::INFINITY), "0");
    }

    #[test]
    fn angles_normalize_into_a_positive_turn() {
        assert_eq!(format_angle_degrees(-90.0), "270");
        assert_eq!(format_angle_degrees(450.0), "90");
        assert_eq!(format_angle_degrees(-1e-12), "0");
        assert_eq!(format_angle_degrees(359.999), "0");
        assert_eq!(format_angle_degrees(90.0), "90");
    }
}
