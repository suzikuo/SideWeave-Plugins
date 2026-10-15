use crate::scroll_input::ScrollInputDelta;

/// State owned by a scrolling capture session. It does not know about the
/// overlay or mouse input; callers feed it frames captured by ScreenCapture.
const MAX_CONSECUTIVE_UNMATCHED: u8 = 10;
const SETTLING_OBSERVATIONS: u8 = 2;
pub(super) const SCROLL_PROBE_GRID: usize = 48;
// Sparse point samples can hit only a small fraction of text during a short scroll.
// The count floor still filters isolated caret or cursor changes.
pub(super) const MIN_CHANGED_PROBE_SAMPLES: usize = 8;
const MATCH_REGION_COUNT: usize = 5;
const MATCH_REGION_MAX_WIDTH: usize = 96;
const MATCH_MIN_OVERLAP_RATIO: usize = 4;
const MATCH_MAX_SHIFT: usize = 1200;
const MATCH_MAX_SCORE: u32 = 2400;
const MATCH_MIN_SCORE_MARGIN: u32 = 250;
const MATCH_SCORE_MARGIN_PERCENT: u32 = 8;
const MATCH_DIRECTION_SCORE_MARGIN: u32 = 500;
const MATCHER_AGREEMENT_TOLERANCE: usize = 3;
const MATCH_PEAK_SEPARATION: usize = 8;
const MATCH_SCORE_ROW_STRIDE: usize = 6;
const OFFSET_HISTORY_MIN_SAMPLES: usize = 3;
const OFFSET_HISTORY_MAX_SAMPLES: usize = 7;
const MOTION_CALIBRATION_MIN_SAMPLES: usize = 3;
const MOTION_CALIBRATION_MAX_SAMPLES: usize = 7;
const MOTION_CALIBRATION_MAX_AGE_MS: u64 = 120_000;
const MOTION_GUIDED_MAX_SCORE: u32 = 900;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CapturePhase {
    Starting,
    Observing,
    Settling,
    Capturing,
    Paused,
    Finishing,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MatchAxis {
    Vertical,
    Horizontal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MotionInputSource {
    Selection,
    TargetWindow,
}

#[derive(Clone, Copy, Debug)]
struct MotionCalibrationSample {
    source: MotionInputSource,
    pixels_per_input_unit: f64,
    tick_ms: u64,
}

#[derive(Default)]
pub(super) struct ScrollMotionModel {
    context: Option<(isize, MatchAxis)>,
    samples: Vec<MotionCalibrationSample>,
}

impl ScrollMotionModel {
    pub(super) fn ensure_context(&mut self, target_window: isize, axis: MatchAxis) {
        let context = (target_window, axis);
        if self.context != Some(context) {
            self.context = Some(context);
            self.samples.clear();
        }
    }

    fn prepare(
        &self,
        input: ScrollInputDelta,
        axis_extent: usize,
    ) -> (MotionDiagnostics, Option<MotionSearchHint>) {
        let (source, input_units) = motion_input(input);
        let estimate = source.and_then(|source| {
            input
                .last_event_tick_ms
                .and_then(|tick_ms| self.estimate(source, tick_ms))
        });
        let matching_source_sample_count = source.map_or(0, |source| {
            self.samples
                .iter()
                .filter(|sample| sample.source == source)
                .count()
        });
        let mut diagnostics = MotionDiagnostics {
            input_source: source,
            input_units_raw: input_units,
            calibration_sample_count: estimate
                .map_or(matching_source_sample_count as u32, |value| value.2 as u32),
            pixels_per_input_unit: estimate.map(|value| value.0),
            predicted_signed_offset: None,
            prediction_tolerance: None,
            guided_search_used: false,
            guided_fallback_reason: None,
            guided_commit: false,
        };
        if source.is_none() {
            return (diagnostics, None);
        }
        if input_units == 0 {
            return (diagnostics, None);
        }
        let Some((pixels_per_unit, deviation, sample_count)) = estimate else {
            return (diagnostics, None);
        };
        let signed_prediction = (input_units as f64 * pixels_per_unit).round();
        if !signed_prediction.is_finite()
            || signed_prediction == 0.0
            || signed_prediction.abs() > axis_extent.saturating_sub(8) as f64
        {
            return (diagnostics, None);
        }
        let predicted_signed_offset = signed_prediction as i32;
        let magnitude = predicted_signed_offset.unsigned_abs() as usize;
        let uncertainty = (input_units.unsigned_abs() as f64 * deviation * 2.0).ceil() as usize;
        let tolerance = (magnitude / 5)
            .max(12)
            .saturating_add(uncertainty)
            .min((magnitude / 2).max(16))
            .min((axis_extent / 3).max(16));
        diagnostics.calibration_sample_count = sample_count as u32;
        diagnostics.predicted_signed_offset = Some(predicted_signed_offset);
        diagnostics.prediction_tolerance = Some(tolerance as u32);
        (
            diagnostics,
            Some(MotionSearchHint {
                signed_offset: predicted_signed_offset,
                tolerance,
            }),
        )
    }

    fn estimate(
        &self,
        source: MotionInputSource,
        current_tick_ms: u64,
    ) -> Option<(f64, f64, usize)> {
        let source_samples = self
            .samples
            .iter()
            .filter(|sample| sample.source == source)
            .collect::<Vec<_>>();
        let newest_tick = source_samples.iter().map(|sample| sample.tick_ms).max()?;
        if current_tick_ms.saturating_sub(newest_tick) > MOTION_CALIBRATION_MAX_AGE_MS {
            return None;
        }
        let ratios = source_samples
            .iter()
            .map(|sample| sample.pixels_per_input_unit)
            .collect::<Vec<_>>();
        if ratios.len() < MOTION_CALIBRATION_MIN_SAMPLES {
            return None;
        }
        let median = median_f64(&ratios)?;
        let deviations = ratios
            .iter()
            .map(|ratio| (ratio - median).abs())
            .collect::<Vec<_>>();
        let deviation = median_f64(&deviations).unwrap_or_default();
        if median.abs() < 0.02 || deviation > median.abs() * 0.25 {
            return None;
        }
        Some((median, deviation, ratios.len()))
    }

    fn estimate_for_input(&self, input: ScrollInputDelta) -> Option<(f64, f64, usize)> {
        let (source, _) = motion_input(input);
        source.and_then(|source| {
            input
                .last_event_tick_ms
                .and_then(|tick_ms| self.estimate(source, tick_ms))
        })
    }

    fn sample_count_for_input(&self, input: ScrollInputDelta) -> usize {
        let (Some(source), _) = motion_input(input) else {
            return 0;
        };
        self.samples
            .iter()
            .filter(|sample| sample.source == source)
            .count()
    }

    fn record_match(
        &mut self,
        input: ScrollInputDelta,
        axis: MatchAxis,
        direction: ScrollDirection,
        shift: usize,
    ) {
        let (Some(source), input_units) = motion_input(input) else {
            return;
        };
        if input_units == 0 || shift == 0 {
            return;
        }
        let Some(tick_ms) = input.last_event_tick_ms else {
            return;
        };
        let signed_shift = match direction {
            ScrollDirection::Down if axis == MatchAxis::Vertical => shift as i64,
            ScrollDirection::Up if axis == MatchAxis::Vertical => -(shift as i64),
            ScrollDirection::Right if axis == MatchAxis::Horizontal => shift as i64,
            ScrollDirection::Left if axis == MatchAxis::Horizontal => -(shift as i64),
            _ => return,
        };
        let ratio = signed_shift as f64 / input_units as f64;
        if !ratio.is_finite() || ratio.abs() < 0.02 || ratio.abs() > 8.0 {
            return;
        }
        if let Some(newest_tick) = self
            .samples
            .iter()
            .filter(|sample| sample.source == source)
            .map(|sample| sample.tick_ms)
            .max()
        {
            if tick_ms.saturating_sub(newest_tick) > MOTION_CALIBRATION_MAX_AGE_MS {
                self.samples.retain(|sample| sample.source != source);
            }
        }
        let recent_ratios = self
            .samples
            .iter()
            .filter(|sample| sample.source == source)
            .map(|sample| sample.pixels_per_input_unit)
            .collect::<Vec<_>>();
        if recent_ratios.len() >= MOTION_CALIBRATION_MIN_SAMPLES {
            if let Some(median) = median_f64(&recent_ratios) {
                if (ratio - median).abs() > (median.abs() * 0.4).max(0.05) {
                    self.samples.retain(|sample| sample.source != source);
                }
            }
        }
        self.samples.push(MotionCalibrationSample {
            source,
            pixels_per_input_unit: ratio,
            tick_ms,
        });
        while self
            .samples
            .iter()
            .filter(|sample| sample.source == source)
            .count()
            > MOTION_CALIBRATION_MAX_SAMPLES
        {
            if let Some(oldest_for_source) = self
                .samples
                .iter()
                .position(|sample| sample.source == source)
            {
                self.samples.remove(oldest_for_source);
            } else {
                break;
            }
        }
    }
}

fn motion_input(input: ScrollInputDelta) -> (Option<MotionInputSource>, i64) {
    match (
        input.axis_units_raw != 0,
        input.target_window_axis_units_raw != 0,
    ) {
        (true, false) => (Some(MotionInputSource::Selection), input.axis_units_raw),
        (false, true) => (
            Some(MotionInputSource::TargetWindow),
            input.target_window_axis_units_raw,
        ),
        // Mixed or unobserved input sources cannot safely train one ratio.
        _ => (None, 0),
    }
}

fn median_f64(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    Some(sorted[sorted.len() / 2])
}

#[derive(Clone, Debug)]
pub(super) struct MotionDiagnostics {
    input_source: Option<MotionInputSource>,
    input_units_raw: i64,
    calibration_sample_count: u32,
    pixels_per_input_unit: Option<f64>,
    predicted_signed_offset: Option<i32>,
    prediction_tolerance: Option<u32>,
    guided_search_used: bool,
    guided_fallback_reason: Option<MatchRejectReason>,
    guided_commit: bool,
}

#[derive(Clone, Copy)]
struct MotionSearchHint {
    signed_offset: i32,
    tolerance: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FrameProbe {
    samples: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ProbeDiff {
    pub(super) changed_samples: usize,
    pub(super) total_samples: usize,
    pub(super) average_delta: u8,
    pub(super) max_delta: u8,
}

impl FrameProbe {
    pub(super) fn new(samples: Vec<u8>) -> Self {
        Self { samples }
    }
    fn diff_from(&self, previous: &Self) -> Option<ProbeDiff> {
        if self.samples.is_empty() || self.samples.len() != previous.samples.len() {
            return None;
        }
        let (mut changed_samples, mut total_delta, mut max_delta) = (0usize, 0usize, 0u8);
        for (current, previous) in self.samples.iter().zip(&previous.samples) {
            let delta = current.abs_diff(*previous);
            if delta >= 7 {
                changed_samples += 1;
            }
            total_delta += delta as usize;
            max_delta = max_delta.max(delta);
        }
        Some(ProbeDiff {
            changed_samples,
            total_samples: self.samples.len(),
            average_delta: (total_delta / self.samples.len()) as u8,
            max_delta,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FrameResult {
    Unchanged,
    MotionDetected,
    Matched { shift: usize, confidence: u16 },
    NoOverlap,
    ReverseDirection { shift: usize, confidence: u16 },
    InvalidFrame,
    ReachedLimit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct MatchRegion {
    pub(super) x: usize,
    pub(super) width: usize,
    pub(super) weight: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MatchRejectReason {
    Unchanged,
    NoOverlap,
    AmbiguousCandidates,
    AmbiguousDirection,
    ReverseDirection,
    HistoricalOffsetOutlier,
    MatcherDisagreement,
    InsufficientStableRegion,
    InvalidFrame,
    AxisNotEvaluated,
    ScoreTooLow,
    SearchRangeBoundary,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CandidateDiagnostics {
    pub(super) search_min: i32,
    pub(super) search_max: i32,
    pub(super) search_range_capped: bool,
    pub(super) hit_search_boundary: bool,
    pub(super) best_offset: Option<i32>,
    /// Mean absolute feature error scaled by 100 (0 is exact).
    pub(super) best_score: Option<u32>,
    pub(super) second_offset: Option<i32>,
    pub(super) second_score: Option<u32>,
    pub(super) candidate_search_elapsed_ms: u128,
    /// Shared pixels along the selected scroll axis (rows vertically, columns horizontally).
    pub(super) overlap_extent: u32,
    pub(super) generated_region_count: u32,
    pub(super) valid_region_count: u32,
    pub(super) supporting_region_count: u32,
    pub(super) ambiguous_region_count: u32,
    pub(super) low_texture_region_count: u32,
    /// Difference between the second score and best score; larger is clearer.
    pub(super) score_margin: Option<u32>,
    pub(super) score_threshold: u32,
    pub(super) margin_threshold: u32,
    pub(super) valid_regions: Vec<MatchRegion>,
    pub(super) supporting_regions: Vec<MatchRegion>,
    pub(super) rejection_reason: Option<MatchRejectReason>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct MatchDiagnostics {
    pub(super) frame_id: u64,
    pub(super) previous_frame_id: Option<u64>,
    pub(super) forward: CandidateDiagnostics,
    pub(super) reverse: CandidateDiagnostics,
    pub(super) horizontal_forward: CandidateDiagnostics,
    pub(super) horizontal_reverse: CandidateDiagnostics,
    pub(super) gradient_forward: Option<CandidateDiagnostics>,
    pub(super) gradient_reverse: Option<CandidateDiagnostics>,
    pub(super) selected_metric: Option<MatchMetric>,
    pub(super) grayscale_preprocess_elapsed_ms: u128,
    pub(super) gradient_preprocess_elapsed_ms: u128,
    pub(super) selected_axis: Option<MatchAxis>,
    pub(super) selected_offset: Option<i32>,
    pub(super) rejection_reason: Option<MatchRejectReason>,
    pub(super) direction_score_margin_threshold: u32,
    pub(super) matcher_elapsed_ms: u128,
    pub(super) stitch_commit_elapsed_ms: u128,
    pub(super) appended_rows: usize,
    pub(super) prepended_rows: usize,
    pub(super) appended_columns: usize,
    pub(super) prepended_columns: usize,
    pub(super) assembled_width_before: usize,
    pub(super) assembled_width_after: usize,
    pub(super) assembled_height_before: usize,
    pub(super) assembled_height_after: usize,
    pub(super) last_valid_frame_id_after: Option<u64>,
    pub(super) recent_offset_count: u32,
    pub(super) recent_offset_median: Option<u32>,
    pub(super) history_reject_threshold: Option<u32>,
    pub(super) rejected_by_offset_history: bool,
    pub(super) stitch_anchor_frame_id_before: Option<u64>,
    pub(super) stitch_anchor_output_right_before: Option<usize>,
    pub(super) stitch_anchor_output_bottom_before: Option<usize>,
    pub(super) stitch_anchor_frame_id_after: Option<u64>,
    pub(super) stitch_anchor_output_right_after: Option<usize>,
    pub(super) stitch_anchor_output_bottom_after: Option<usize>,
}

impl CandidateDiagnostics {
    fn empty(reason: MatchRejectReason) -> Self {
        Self {
            search_min: 0,
            search_max: 0,
            search_range_capped: false,
            hit_search_boundary: false,
            best_offset: None,
            best_score: None,
            second_offset: None,
            second_score: None,
            candidate_search_elapsed_ms: 0,
            overlap_extent: 0,
            generated_region_count: 0,
            valid_region_count: 0,
            supporting_region_count: 0,
            ambiguous_region_count: 0,
            low_texture_region_count: 0,
            score_margin: None,
            score_threshold: MATCH_MAX_SCORE,
            margin_threshold: MATCH_MIN_SCORE_MARGIN,
            valid_regions: Vec::new(),
            supporting_regions: Vec::new(),
            rejection_reason: Some(reason),
        }
    }
}

struct CandidateMatch {
    shift: Option<usize>,
    confidence: u16,
    diagnostics: CandidateDiagnostics,
}

struct AxisCandidates {
    forward: CandidateMatch,
    reverse: CandidateMatch,
    gradient_forward: Option<CandidateMatch>,
    gradient_reverse: Option<CandidateMatch>,
    selected_direction: Option<MatchDirection>,
    selected_metric: MatchMetric,
    shift: Option<usize>,
    confidence: u16,
    motion_guided: bool,
    motion_guided_fallback_reason: Option<MatchRejectReason>,
    motion_calibration_eligible: bool,
    grayscale_preprocess_elapsed_ms: u128,
    gradient_preprocess_elapsed_ms: u128,
    rejection_reason: Option<MatchRejectReason>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MatchMetric {
    Grayscale,
    Gradient,
    CrossValidated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MatchDirection {
    Forward,
    Reverse,
}

struct RegionScores {
    region: MatchRegion,
    best_score: u32,
    ambiguous: bool,
    scores: Vec<u32>,
}

/// `last_valid_frame` is the latest high-quality frame accepted by Matcher.
/// `stitch_anchor` records that frame's right and bottom edges in the assembled image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct StitchAnchor {
    pub(super) frame_id: u64,
    /// Output-space coordinate of the last_valid_frame's right edge.
    pub(super) output_right: usize,
    /// Output-space coordinate of the last_valid_frame's bottom edge.
    pub(super) output_bottom: usize,
}

pub(super) struct ScrollingCapture {
    pub(super) assembled: Option<(usize, usize, Vec<u8>)>,
    /// Most recent validated frame used as the next match baseline.
    pub(super) last_valid_frame: Option<Vec<u8>>,
    pub(super) last_valid_frame_id: Option<u64>,
    /// Output-space location corresponding to the edges of last_valid_frame.
    pub(super) stitch_anchor: Option<StitchAnchor>,
    pub(super) last_match_diagnostics: Option<MatchDiagnostics>,
    pub(super) last_motion_diagnostics: Option<MotionDiagnostics>,
    /// Input accumulated since `last_valid_frame`; a later match spans this
    /// entire interval, so its motion calibration must use the same interval.
    pending_motion_input: ScrollInputDelta,
    pub(super) idle_ticks: u8,
    pub(super) unmatched_ticks: u8,
    /// True after at least one frame has been appended beyond the seed frame.
    pub(super) progressed: bool,
    pub(super) phase: CapturePhase,
    pub(super) direction: Option<ScrollDirection>,
    pub(super) match_axis: MatchAxis,
    pub(super) last_probe_diff: Option<ProbeDiff>,
    probe: Option<FrameProbe>,
    last_probe: Option<FrameProbe>,
    stable_ticks: u8,
    next_frame_id: u64,
    accepted_offset_history: Vec<usize>,
}

impl ScrollingCapture {
    pub(super) fn pending() -> Self {
        Self {
            assembled: None,
            last_valid_frame: None,
            last_valid_frame_id: None,
            stitch_anchor: None,
            last_match_diagnostics: None,
            last_motion_diagnostics: None,
            pending_motion_input: ScrollInputDelta::default(),
            idle_ticks: 0,
            unmatched_ticks: 0,
            progressed: false,
            phase: CapturePhase::Starting,
            direction: None,
            match_axis: MatchAxis::Vertical,
            last_probe_diff: None,
            probe: None,
            last_probe: None,
            stable_ticks: 0,
            next_frame_id: 1,
            accepted_offset_history: Vec::new(),
        }
    }

    pub(super) fn seed(&mut self, width: usize, height: usize, frame: Vec<u8>) {
        if self.assembled.is_none() {
            self.assembled = Some((width, height, frame.clone()));
            self.last_valid_frame = Some(frame);
            self.last_valid_frame_id = Some(0);
            self.stitch_anchor = Some(StitchAnchor {
                frame_id: 0,
                output_right: width,
                output_bottom: height,
            });
            self.last_match_diagnostics = None;
            self.pending_motion_input = ScrollInputDelta::default();
            self.next_frame_id = 1;
            self.accepted_offset_history.clear();
            self.direction = None;
            self.idle_ticks = 0;
            self.unmatched_ticks = 0;
            self.progressed = false;
            self.phase = CapturePhase::Observing;
        }
    }

    pub(super) fn set_axis(&mut self, axis: MatchAxis) {
        if self.match_axis != axis {
            self.pending_motion_input = ScrollInputDelta::default();
        }
        self.match_axis = axis;
    }

    pub(super) fn seed_probe(&mut self, probe: FrameProbe) {
        self.probe = Some(probe.clone());
        self.last_probe = Some(probe);
        self.last_probe_diff = None;
        if self.phase == CapturePhase::Starting {
            self.phase = CapturePhase::Observing;
        }
    }

    pub(super) fn observe_probe(&mut self, probe: FrameProbe) -> FrameResult {
        self.last_probe_diff = None;
        let Some(baseline) = self.probe.as_ref() else {
            self.seed_probe(probe);
            return FrameResult::Unchanged;
        };
        // In Observing, compare against the last committed stable frame so
        // several small movements can accumulate into one detected change.
        // During Settling, compare adjacent probes to track ongoing motion;
        // the UI may attempt a fast match on each detected change, while only
        // a visually verified result is allowed to advance the stitched image.
        let reference =
            if self.phase == CapturePhase::Settling || self.phase == CapturePhase::Capturing {
                self.last_probe.as_ref().unwrap_or(baseline)
            } else {
                baseline
            };
        let Some(diff) = probe.diff_from(reference) else {
            self.phase = CapturePhase::Paused;
            return FrameResult::InvalidFrame;
        };
        self.last_probe_diff = Some(diff);
        let changed = diff.changed_samples >= MIN_CHANGED_PROBE_SAMPLES;
        if changed {
            self.last_probe = Some(probe);
            self.phase = CapturePhase::Settling;
            self.stable_ticks = 0;
            return FrameResult::MotionDetected;
        }
        if self.phase == CapturePhase::Settling || self.phase == CapturePhase::Capturing {
            self.last_probe = Some(probe);
        }
        if self.phase == CapturePhase::Settling {
            self.stable_ticks = self.stable_ticks.saturating_add(1);
            if self.stable_ticks >= SETTLING_OBSERVATIONS {
                self.phase = CapturePhase::Capturing;
            }
        }
        FrameResult::Unchanged
    }

    pub(super) fn commit_probe(&mut self) {
        if let Some(probe) = self.last_probe.clone() {
            self.probe = Some(probe);
        }
    }

    pub(super) fn finish(&mut self) {
        self.phase = CapturePhase::Finishing;
    }

    /// Pause after an external capture failure without ending the session.
    /// Commit the newest probe so a later user scroll can resume from the
    /// latest observed content instead of repeatedly retrying the same frame.
    pub(super) fn pause_after_capture_failure(&mut self) {
        self.pause_after_capture_failure_with_input(ScrollInputDelta::default());
    }

    pub(super) fn pause_after_capture_failure_with_input(
        &mut self,
        input_delta: ScrollInputDelta,
    ) {
        self.pending_motion_input.merge(input_delta);
        self.commit_probe();
        self.stable_ticks = 0;
        self.phase = CapturePhase::Paused;
    }

    pub(super) fn ready_to_capture(&self) -> bool {
        self.phase == CapturePhase::Capturing
    }
    pub(super) fn next_capture_frame_id(&mut self) -> u64 {
        let frame_id = self.next_frame_id;
        self.next_frame_id = self.next_frame_id.saturating_add(1);
        frame_id
    }

    pub(super) fn capture_stable(
        &mut self,
        current: Vec<u8>,
        width: usize,
        height: usize,
        max_extent: usize,
    ) -> FrameResult {
        let frame_id = self.next_capture_frame_id();
        self.capture_stable_with_id(current, width, height, max_extent, frame_id)
    }

    pub(super) fn capture_stable_with_id(
        &mut self,
        current: Vec<u8>,
        width: usize,
        height: usize,
        max_extent: usize,
        frame_id: u64,
    ) -> FrameResult {
        let mut motion_model = ScrollMotionModel::default();
        self.capture_stable_with_id_and_motion(
            current,
            width,
            height,
            max_extent,
            frame_id,
            ScrollInputDelta::default(),
            &mut motion_model,
        )
    }

    pub(super) fn capture_stable_with_id_and_motion(
        &mut self,
        current: Vec<u8>,
        width: usize,
        height: usize,
        max_extent: usize,
        frame_id: u64,
        input_delta: ScrollInputDelta,
        motion_model: &mut ScrollMotionModel,
    ) -> FrameResult {
        use std::time::Instant;

        let matcher_started = Instant::now();
        let axis = self.match_axis;
        self.pending_motion_input.merge(input_delta);
        let input_since_last_valid_frame = self.pending_motion_input;
        let axis_extent = match axis {
            MatchAxis::Vertical => height,
            MatchAxis::Horizontal => width,
        };
        let (mut motion_diagnostics, motion_hint) =
            motion_model.prepare(input_since_last_valid_frame, axis_extent);
        self.last_motion_diagnostics = Some(motion_diagnostics.clone());
        let assembled_height_before = self
            .assembled
            .as_ref()
            .map(|(_, assembled_height, _)| *assembled_height)
            .unwrap_or_default();
        let previous_frame_id = self.last_valid_frame_id;
        let Some(previous) = self.last_valid_frame.as_deref() else {
            self.phase = CapturePhase::Paused;
            self.last_match_diagnostics = Some(MatchDiagnostics {
                frame_id,
                previous_frame_id,
                forward: CandidateDiagnostics::empty(MatchRejectReason::InvalidFrame),
                reverse: CandidateDiagnostics::empty(MatchRejectReason::InvalidFrame),
                horizontal_forward: CandidateDiagnostics::empty(MatchRejectReason::AxisNotEvaluated),
                horizontal_reverse: CandidateDiagnostics::empty(MatchRejectReason::AxisNotEvaluated),
                gradient_forward: None,
                gradient_reverse: None,
                selected_metric: None,
                grayscale_preprocess_elapsed_ms: 0,
                gradient_preprocess_elapsed_ms: 0,
                selected_axis: Some(self.match_axis),
                selected_offset: None,
                rejection_reason: Some(MatchRejectReason::InvalidFrame),
                direction_score_margin_threshold: MATCH_DIRECTION_SCORE_MARGIN,
                matcher_elapsed_ms: matcher_started.elapsed().as_millis(),
                stitch_commit_elapsed_ms: 0,
                appended_rows: 0,
                prepended_rows: 0,
                appended_columns: 0,
                prepended_columns: 0,
                assembled_width_before: self.assembled.as_ref().map(|(width, _, _)| *width).unwrap_or_default(),
                assembled_width_after: self.assembled.as_ref().map(|(width, _, _)| *width).unwrap_or_default(),
                assembled_height_before,
                assembled_height_after: assembled_height_before,
                last_valid_frame_id_after: self.last_valid_frame_id,
                recent_offset_count: self.accepted_offset_history.len() as u32,
                recent_offset_median: offset_history_median(&self.accepted_offset_history)
                    .map(|offset| offset as u32),
                history_reject_threshold: offset_history_reject_threshold(
                    &self.accepted_offset_history,
                )
                .map(|offset| offset as u32),
                rejected_by_offset_history: false,
                stitch_anchor_frame_id_before: self.stitch_anchor.map(|anchor| anchor.frame_id),
                stitch_anchor_output_right_before: self.stitch_anchor.map(|anchor| anchor.output_right),
                stitch_anchor_output_bottom_before: self
                    .stitch_anchor
                    .map(|anchor| anchor.output_bottom),
                stitch_anchor_frame_id_after: self.stitch_anchor.map(|anchor| anchor.frame_id),
                stitch_anchor_output_right_after: self.stitch_anchor.map(|anchor| anchor.output_right),
                stitch_anchor_output_bottom_after: self
                    .stitch_anchor
                    .map(|anchor| anchor.output_bottom),
            });
            return FrameResult::InvalidFrame;
        };
        let expected = width.saturating_mul(height).saturating_mul(4);
        if expected == 0 || current.len() != expected || previous.len() != expected {
            self.phase = CapturePhase::Paused;
            self.last_match_diagnostics = Some(MatchDiagnostics {
                frame_id,
                previous_frame_id,
                forward: CandidateDiagnostics::empty(MatchRejectReason::InvalidFrame),
                reverse: CandidateDiagnostics::empty(MatchRejectReason::InvalidFrame),
                horizontal_forward: CandidateDiagnostics::empty(MatchRejectReason::AxisNotEvaluated),
                horizontal_reverse: CandidateDiagnostics::empty(MatchRejectReason::AxisNotEvaluated),
                gradient_forward: None,
                gradient_reverse: None,
                selected_metric: None,
                grayscale_preprocess_elapsed_ms: 0,
                gradient_preprocess_elapsed_ms: 0,
                selected_axis: Some(self.match_axis),
                selected_offset: None,
                rejection_reason: Some(MatchRejectReason::InvalidFrame),
                direction_score_margin_threshold: MATCH_DIRECTION_SCORE_MARGIN,
                matcher_elapsed_ms: matcher_started.elapsed().as_millis(),
                stitch_commit_elapsed_ms: 0,
                appended_rows: 0,
                prepended_rows: 0,
                appended_columns: 0,
                prepended_columns: 0,
                assembled_width_before: self.assembled.as_ref().map(|(width, _, _)| *width).unwrap_or_default(),
                assembled_width_after: self.assembled.as_ref().map(|(width, _, _)| *width).unwrap_or_default(),
                assembled_height_before,
                assembled_height_after: assembled_height_before,
                last_valid_frame_id_after: self.last_valid_frame_id,
                recent_offset_count: self.accepted_offset_history.len() as u32,
                recent_offset_median: offset_history_median(&self.accepted_offset_history)
                    .map(|offset| offset as u32),
                history_reject_threshold: offset_history_reject_threshold(
                    &self.accepted_offset_history,
                )
                .map(|offset| offset as u32),
                rejected_by_offset_history: false,
                stitch_anchor_frame_id_before: self.stitch_anchor.map(|anchor| anchor.frame_id),
                stitch_anchor_output_right_before: self.stitch_anchor.map(|anchor| anchor.output_right),
                stitch_anchor_output_bottom_before: self
                    .stitch_anchor
                    .map(|anchor| anchor.output_bottom),
                stitch_anchor_frame_id_after: self.stitch_anchor.map(|anchor| anchor.frame_id),
                stitch_anchor_output_right_after: self.stitch_anchor.map(|anchor| anchor.output_right),
                stitch_anchor_output_bottom_after: self
                    .stitch_anchor
                    .map(|anchor| anchor.output_bottom),
            });
            return FrameResult::InvalidFrame;
        }
        if current == previous {
            self.commit_probe();
            self.phase = CapturePhase::Observing;
            self.last_match_diagnostics = Some(MatchDiagnostics {
                frame_id,
                previous_frame_id,
                forward: CandidateDiagnostics::empty(MatchRejectReason::Unchanged),
                reverse: CandidateDiagnostics::empty(MatchRejectReason::Unchanged),
                horizontal_forward: CandidateDiagnostics::empty(MatchRejectReason::AxisNotEvaluated),
                horizontal_reverse: CandidateDiagnostics::empty(MatchRejectReason::AxisNotEvaluated),
                gradient_forward: None,
                gradient_reverse: None,
                selected_metric: None,
                grayscale_preprocess_elapsed_ms: 0,
                gradient_preprocess_elapsed_ms: 0,
                selected_axis: Some(self.match_axis),
                selected_offset: None,
                rejection_reason: None,
                direction_score_margin_threshold: MATCH_DIRECTION_SCORE_MARGIN,
                matcher_elapsed_ms: matcher_started.elapsed().as_millis(),
                stitch_commit_elapsed_ms: 0,
                appended_rows: 0,
                prepended_rows: 0,
                appended_columns: 0,
                prepended_columns: 0,
                assembled_width_before: self.assembled.as_ref().map(|(width, _, _)| *width).unwrap_or_default(),
                assembled_width_after: self.assembled.as_ref().map(|(width, _, _)| *width).unwrap_or_default(),
                assembled_height_before,
                assembled_height_after: assembled_height_before,
                last_valid_frame_id_after: self.last_valid_frame_id,
                recent_offset_count: self.accepted_offset_history.len() as u32,
                recent_offset_median: offset_history_median(&self.accepted_offset_history)
                    .map(|offset| offset as u32),
                history_reject_threshold: offset_history_reject_threshold(
                    &self.accepted_offset_history,
                )
                .map(|offset| offset as u32),
                rejected_by_offset_history: false,
                stitch_anchor_frame_id_before: self.stitch_anchor.map(|anchor| anchor.frame_id),
                stitch_anchor_output_right_before: self.stitch_anchor.map(|anchor| anchor.output_right),
                stitch_anchor_output_bottom_before: self
                    .stitch_anchor
                    .map(|anchor| anchor.output_bottom),
                stitch_anchor_frame_id_after: self.stitch_anchor.map(|anchor| anchor.frame_id),
                stitch_anchor_output_right_after: self.stitch_anchor.map(|anchor| anchor.output_right),
                stitch_anchor_output_bottom_after: self
                    .stitch_anchor
                    .map(|anchor| anchor.output_bottom),
            });
            return FrameResult::Unchanged;
        }

        let candidates = find_axis_candidates(previous, &current, width, height, axis, motion_hint);
        motion_diagnostics.guided_search_used =
            candidates.motion_guided || candidates.motion_guided_fallback_reason.is_some();
        motion_diagnostics.guided_fallback_reason = candidates.motion_guided_fallback_reason;
        motion_diagnostics.guided_commit = candidates.motion_guided && candidates.shift.is_some();
        self.last_motion_diagnostics = Some(motion_diagnostics.clone());
        let matcher_elapsed_ms = matcher_started.elapsed().as_millis();
        let selected_direction = candidates.selected_direction;
        let shift = candidates.shift;
        let confidence = candidates.confidence;
        let selected_metric = candidates.selected_metric;
        let grayscale_preprocess_elapsed_ms = candidates.grayscale_preprocess_elapsed_ms;
        let gradient_preprocess_elapsed_ms = candidates.gradient_preprocess_elapsed_ms;
        let rejection_reason = candidates.rejection_reason;
        let motion_guided = candidates.motion_guided;
        let motion_calibration_eligible = candidates.motion_calibration_eligible;
        let (
            forward,
            reverse,
            horizontal_forward,
            horizontal_reverse,
            gradient_forward,
            gradient_reverse,
        ) = match axis {
            MatchAxis::Vertical => (
                candidates.forward.diagnostics,
                candidates.reverse.diagnostics,
                CandidateDiagnostics::empty(MatchRejectReason::AxisNotEvaluated),
                CandidateDiagnostics::empty(MatchRejectReason::AxisNotEvaluated),
                candidates.gradient_forward.map(|candidate| candidate.diagnostics),
                candidates.gradient_reverse.map(|candidate| candidate.diagnostics),
            ),
            MatchAxis::Horizontal => (
                CandidateDiagnostics::empty(MatchRejectReason::AxisNotEvaluated),
                CandidateDiagnostics::empty(MatchRejectReason::AxisNotEvaluated),
                candidates.forward.diagnostics,
                candidates.reverse.diagnostics,
                candidates.gradient_forward.map(|candidate| candidate.diagnostics),
                candidates.gradient_reverse.map(|candidate| candidate.diagnostics),
            ),
        };
        let mut diagnostics = MatchDiagnostics {
            frame_id,
            previous_frame_id,
            forward,
            reverse,
            horizontal_forward,
            horizontal_reverse,
            gradient_forward,
            gradient_reverse,
            selected_metric: shift.map(|_| selected_metric),
            grayscale_preprocess_elapsed_ms,
            gradient_preprocess_elapsed_ms,
            selected_axis: Some(axis),
            selected_offset: shift.map(|value| {
                if selected_direction == Some(MatchDirection::Reverse) {
                    -(value as i32)
                } else {
                    value as i32
                }
            }),
            rejection_reason,
            direction_score_margin_threshold: MATCH_DIRECTION_SCORE_MARGIN,
            matcher_elapsed_ms,
            stitch_commit_elapsed_ms: 0,
            appended_rows: 0,
            prepended_rows: 0,
            appended_columns: 0,
            prepended_columns: 0,
            assembled_width_before: self.assembled.as_ref().map(|(w, _, _)| *w).unwrap_or_default(),
            assembled_width_after: self.assembled.as_ref().map(|(w, _, _)| *w).unwrap_or_default(),
            assembled_height_before,
            assembled_height_after: assembled_height_before,
            last_valid_frame_id_after: self.last_valid_frame_id,
            recent_offset_count: self.accepted_offset_history.len() as u32,
            recent_offset_median: offset_history_median(&self.accepted_offset_history)
                .map(|offset| offset as u32),
            history_reject_threshold: offset_history_reject_threshold(
                &self.accepted_offset_history,
            )
            .map(|offset| offset as u32),
            rejected_by_offset_history: false,
            stitch_anchor_frame_id_before: self.stitch_anchor.map(|anchor| anchor.frame_id),
            stitch_anchor_output_right_before: self.stitch_anchor.map(|anchor| anchor.output_right),
            stitch_anchor_output_bottom_before: self
                .stitch_anchor
                .map(|anchor| anchor.output_bottom),
            stitch_anchor_frame_id_after: self.stitch_anchor.map(|anchor| anchor.frame_id),
            stitch_anchor_output_right_after: self.stitch_anchor.map(|anchor| anchor.output_right),
            stitch_anchor_output_bottom_after: self
                .stitch_anchor
                .map(|anchor| anchor.output_bottom),
        };

        let Some(shift) = shift else {
            self.commit_probe();
            self.phase = CapturePhase::Paused;
            self.last_match_diagnostics = Some(diagnostics);
            return FrameResult::NoOverlap;
        };

        if let Some(threshold) =
            offset_history_reject_threshold(&self.accepted_offset_history)
        {
            diagnostics.history_reject_threshold = Some(threshold as u32);
            if shift > threshold && !motion_guided {
                diagnostics.rejection_reason = Some(MatchRejectReason::HistoricalOffsetOutlier);
                let candidate_diagnostics = match (
                    axis,
                    selected_metric,
                    selected_direction,
                ) {
                    (
                        MatchAxis::Vertical,
                        MatchMetric::Grayscale | MatchMetric::CrossValidated,
                        Some(MatchDirection::Reverse),
                    ) => {
                        Some(&mut diagnostics.reverse)
                    }
                    (MatchAxis::Vertical, MatchMetric::Gradient, Some(MatchDirection::Forward)) => {
                        diagnostics.gradient_forward.as_mut()
                    }
                    (MatchAxis::Vertical, MatchMetric::Gradient, Some(MatchDirection::Reverse)) => {
                        diagnostics.gradient_reverse.as_mut()
                    }
                    (
                        MatchAxis::Horizontal,
                        MatchMetric::Grayscale | MatchMetric::CrossValidated,
                        Some(MatchDirection::Forward),
                    ) => {
                        Some(&mut diagnostics.horizontal_forward)
                    }
                    (
                        MatchAxis::Horizontal,
                        MatchMetric::Grayscale | MatchMetric::CrossValidated,
                        Some(MatchDirection::Reverse),
                    ) => {
                        Some(&mut diagnostics.horizontal_reverse)
                    }
                    (MatchAxis::Horizontal, MatchMetric::Gradient, Some(MatchDirection::Forward)) => {
                        diagnostics.gradient_forward.as_mut()
                    }
                    (MatchAxis::Horizontal, MatchMetric::Gradient, Some(MatchDirection::Reverse)) => {
                        diagnostics.gradient_reverse.as_mut()
                    }
                    (
                        MatchAxis::Vertical,
                        MatchMetric::Grayscale | MatchMetric::CrossValidated,
                        _,
                    ) => {
                        Some(&mut diagnostics.forward)
                    }
                    (
                        MatchAxis::Horizontal,
                        MatchMetric::Grayscale | MatchMetric::CrossValidated,
                        _,
                    ) => {
                        Some(&mut diagnostics.horizontal_forward)
                    }
                    (MatchAxis::Vertical, MatchMetric::Gradient, _) => {
                        diagnostics.gradient_forward.as_mut()
                    }
                    (MatchAxis::Horizontal, MatchMetric::Gradient, _) => {
                        diagnostics.gradient_forward.as_mut()
                    }
                };
                if let Some(candidate_diagnostics) = candidate_diagnostics {
                    candidate_diagnostics.rejection_reason =
                        Some(MatchRejectReason::HistoricalOffsetOutlier);
                }
                diagnostics.rejected_by_offset_history = true;
                self.commit_probe();
                self.phase = CapturePhase::Paused;
                self.last_match_diagnostics = Some(diagnostics);
                return FrameResult::NoOverlap;
            }
        }

        let Some(selected_direction) = selected_direction else {
            self.commit_probe();
            self.phase = CapturePhase::Paused;
            diagnostics.rejection_reason = Some(MatchRejectReason::AmbiguousDirection);
            self.last_match_diagnostics = Some(diagnostics);
            return FrameResult::NoOverlap;
        };
        let direction = match (axis, selected_direction) {
            (MatchAxis::Vertical, MatchDirection::Forward) => ScrollDirection::Down,
            (MatchAxis::Vertical, MatchDirection::Reverse) => ScrollDirection::Up,
            (MatchAxis::Horizontal, MatchDirection::Forward) => ScrollDirection::Right,
            (MatchAxis::Horizontal, MatchDirection::Reverse) => ScrollDirection::Left,
        };
        let commit_started = Instant::now();
        let observation = self.append_validated_frame(
            current,
            width,
            height,
            shift,
            axis,
            direction,
            frame_id,
            max_extent,
        );
        if matches!(observation, ScrollObservation::Advanced)
            && motion_calibration_eligible
            && !motion_guided
        {
            motion_model.record_match(input_since_last_valid_frame, axis, direction, shift);
            if let Some(diagnostics) = self.last_motion_diagnostics.as_mut() {
                diagnostics.calibration_sample_count =
                    motion_model.sample_count_for_input(input_since_last_valid_frame) as u32;
                diagnostics.pixels_per_input_unit = motion_model
                    .estimate_for_input(input_since_last_valid_frame)
                    .map(|estimate| estimate.0);
            }
        }
        if self.last_valid_frame_id == Some(frame_id) {
            self.pending_motion_input = ScrollInputDelta::default();
        }
        diagnostics.stitch_commit_elapsed_ms = commit_started.elapsed().as_millis();
        let (assembled_width_after, assembled_height_after) = self
            .assembled
            .as_ref()
            .map(|(assembled_width, assembled_height, _)| (*assembled_width, *assembled_height))
            .unwrap_or((diagnostics.assembled_width_before, assembled_height_before));
        diagnostics.assembled_width_after = assembled_width_after;
        diagnostics.assembled_height_after = assembled_height_after;
        let changed_rows = assembled_height_after.saturating_sub(assembled_height_before);
        let changed_columns = assembled_width_after.saturating_sub(diagnostics.assembled_width_before);
        match direction {
            ScrollDirection::Down => diagnostics.appended_rows = changed_rows,
            ScrollDirection::Up => diagnostics.prepended_rows = changed_rows,
            ScrollDirection::Right => diagnostics.appended_columns = changed_columns,
            ScrollDirection::Left => diagnostics.prepended_columns = changed_columns,
        }
        diagnostics.last_valid_frame_id_after = self.last_valid_frame_id;
        diagnostics.stitch_anchor_frame_id_after = self.stitch_anchor.map(|anchor| anchor.frame_id);
        diagnostics.stitch_anchor_output_right_after =
            self.stitch_anchor.map(|anchor| anchor.output_right);
        diagnostics.stitch_anchor_output_bottom_after =
            self.stitch_anchor.map(|anchor| anchor.output_bottom);

        let result = match observation {
            ScrollObservation::Advanced => {
                self.commit_probe();
                // Return to the cheap observer after one validated frame. The
                // next high-quality capture remains gated by fresh motion and settling.
                self.phase = CapturePhase::Observing;
                FrameResult::Matched { shift, confidence }
            }
            ScrollObservation::ReachedLimit => {
                self.commit_probe();
                self.phase = CapturePhase::Paused;
                FrameResult::ReachedLimit
            }
            ScrollObservation::NoSession => {
                self.commit_probe();
                self.phase = CapturePhase::Paused;
                diagnostics.rejection_reason = Some(MatchRejectReason::InvalidFrame);
                FrameResult::InvalidFrame
            }
            ScrollObservation::Idle { .. } => {
                self.commit_probe();
                self.phase = CapturePhase::Paused;
                diagnostics.rejection_reason = Some(MatchRejectReason::NoOverlap);
                FrameResult::NoOverlap
            }
        };
        self.last_match_diagnostics = Some(diagnostics);
        result
    }

    fn append_validated_frame(
        &mut self,
        current: Vec<u8>,
        width: usize,
        height: usize,
        shift: usize,
        axis: MatchAxis,
        direction: ScrollDirection,
        frame_id: u64,
        max_extent: usize,
    ) -> ScrollObservation {
        let Some((assembled_width, assembled_height, assembled)) = self.assembled.as_mut() else {
            return ScrollObservation::NoSession;
        };
        if *assembled_width == 0 || *assembled_height == 0 || width == 0 || height == 0 {
            return ScrollObservation::NoSession;
        }

        let Some(previous_anchor) = self.stitch_anchor else {
            return ScrollObservation::NoSession;
        };
        let mut changed_rows = 0usize;
        let mut changed_columns = 0usize;
        let mut reached_limit = false;
        let mut output_right = previous_anchor.output_right;
        let mut output_bottom = previous_anchor.output_bottom;

        match (axis, direction) {
            (MatchAxis::Vertical, ScrollDirection::Down | ScrollDirection::Up) => {
                if *assembled_width != width {
                    return ScrollObservation::NoSession;
                }
                let target_bottom = if direction == ScrollDirection::Down {
                    previous_anchor.output_bottom.saturating_add(shift)
                } else {
                    previous_anchor.output_bottom.saturating_sub(shift)
                };
                let prefix_rows = height.saturating_sub(target_bottom);
                let desired_append_rows = target_bottom.saturating_sub(*assembled_height);
                let available = max_extent.saturating_sub(*assembled_height);
                if prefix_rows > 0 {
                    if prefix_rows > available {
                        return ScrollObservation::ReachedLimit;
                    }
                    let prefix_bytes = prefix_rows.saturating_mul(width).saturating_mul(4);
                    let mut expanded = Vec::with_capacity(assembled.len().saturating_add(prefix_bytes));
                    expanded.extend_from_slice(&current[..prefix_bytes]);
                    expanded.extend_from_slice(assembled);
                    *assembled = expanded;
                    *assembled_height += prefix_rows;
                    changed_rows = prefix_rows;
                } else if desired_append_rows > 0 {
                    let accepted_rows = desired_append_rows.min(available);
                    if accepted_rows == 0 {
                        reached_limit = true;
                    } else {
                        let source_start_row = height.saturating_sub(desired_append_rows);
                        let source_end_row = source_start_row.saturating_add(accepted_rows).min(height);
                        let source_start = source_start_row.saturating_mul(width).saturating_mul(4);
                        let source_end = source_end_row.saturating_mul(width).saturating_mul(4);
                        assembled.extend_from_slice(&current[source_start..source_end]);
                        *assembled_height += accepted_rows;
                        changed_rows = accepted_rows;
                        reached_limit = accepted_rows < desired_append_rows;
                    }
                }
                output_bottom = if direction == ScrollDirection::Down {
                    target_bottom
                } else {
                    target_bottom.saturating_add(prefix_rows)
                };
            }
            (MatchAxis::Horizontal, ScrollDirection::Right | ScrollDirection::Left) => {
                if *assembled_height != height {
                    return ScrollObservation::NoSession;
                }
                let target_right = if direction == ScrollDirection::Right {
                    previous_anchor.output_right.saturating_add(shift)
                } else {
                    previous_anchor.output_right.saturating_sub(shift)
                };
                let prefix_columns = width.saturating_sub(target_right);
                let desired_append_columns = target_right.saturating_sub(*assembled_width);
                let available = max_extent.saturating_sub(*assembled_width);
                if prefix_columns > 0 {
                    if prefix_columns > available {
                        return ScrollObservation::ReachedLimit;
                    }
                    let old_width = *assembled_width;
                    let new_width = old_width + prefix_columns;
                    let Some(expanded_len) = new_width.checked_mul(height).and_then(|n| n.checked_mul(4)) else {
                        return ScrollObservation::NoSession;
                    };
                    let mut expanded = vec![0; expanded_len];
                    for y in 0..height {
                        let row_bytes = y * width * 4;
                        let old_row = y * old_width * 4;
                        let target_row = y * new_width * 4;
                        expanded[target_row..target_row + prefix_columns * 4]
                            .copy_from_slice(&current[row_bytes..row_bytes + prefix_columns * 4]);
                        expanded[target_row + prefix_columns * 4..target_row + new_width * 4]
                            .copy_from_slice(&assembled[old_row..old_row + old_width * 4]);
                    }
                    *assembled = expanded;
                    *assembled_width = new_width;
                    changed_columns = prefix_columns;
                } else if desired_append_columns > 0 {
                    let accepted_columns = desired_append_columns.min(available);
                    if accepted_columns == 0 {
                        reached_limit = true;
                    } else {
                        let old_width = *assembled_width;
                        let new_width = old_width + accepted_columns;
                        let source_start_column = width.saturating_sub(desired_append_columns);
                        let source_start = source_start_column * 4;
                        let source_end = (source_start_column + accepted_columns).min(width) * 4;
                        let Some(expanded_len) = new_width.checked_mul(height).and_then(|n| n.checked_mul(4)) else {
                            return ScrollObservation::NoSession;
                        };
                        let mut expanded = vec![0; expanded_len];
                        for y in 0..height {
                            let old_row = y * old_width * 4;
                            let target_row = y * new_width * 4;
                            let source_row = y * width * 4;
                            expanded[target_row..target_row + old_width * 4]
                                .copy_from_slice(&assembled[old_row..old_row + old_width * 4]);
                            expanded[target_row + old_width * 4..target_row + new_width * 4]
                                .copy_from_slice(&current[source_row + source_start..source_row + source_end]);
                        }
                        *assembled = expanded;
                        *assembled_width = new_width;
                        changed_columns = accepted_columns;
                        reached_limit = accepted_columns < desired_append_columns;
                    }
                }
                output_right = if direction == ScrollDirection::Right {
                    target_right
                } else {
                    target_right.saturating_add(prefix_columns)
                };
            }
            _ => return ScrollObservation::NoSession,
        }

        if reached_limit {
            return ScrollObservation::ReachedLimit;
        }

        self.last_valid_frame = Some(current);
        self.last_valid_frame_id = Some(frame_id);
        self.stitch_anchor = Some(StitchAnchor {
            frame_id,
            output_right,
            output_bottom,
        });
        self.idle_ticks = 0;
        self.unmatched_ticks = 0;
        self.progressed |= changed_rows > 0 || changed_columns > 0;
        self.accepted_offset_history.push(shift);
        if self.accepted_offset_history.len() > OFFSET_HISTORY_MAX_SAMPLES {
            self.accepted_offset_history.remove(0);
        }
        self.direction = Some(direction);
        self.match_axis = axis;

        let axis_extent = match axis {
            MatchAxis::Vertical => *assembled_height,
            MatchAxis::Horizontal => *assembled_width,
        };
        if axis_extent >= max_extent {
            ScrollObservation::ReachedLimit
        } else {
            ScrollObservation::Advanced
        }
    }
    pub(super) fn observe(
        &mut self,
        current: Vec<u8>,
        width: usize,
        height: usize,
        max_extent: usize,
    ) -> ScrollObservation {
        let Some(previous) = self.last_valid_frame.as_deref() else {
            self.seed(width, height, current);
            return ScrollObservation::Advanced;
        };
        let expected = width.saturating_mul(height).saturating_mul(4);
        if expected == 0 || current.len() != expected || previous.len() != expected {
            return self.record_unmatched();
        }

        if current == previous {
            self.unmatched_ticks = 0;
            self.idle_ticks = self.idle_ticks.saturating_add(1);
            return ScrollObservation::Idle {
                reached_idle_limit: false,
            };
        }

        let axis = self.match_axis;
        let candidates = find_axis_candidates(previous, &current, width, height, axis, None);
        let Some(shift) = candidates.shift else {
            // Keep the last validated frame and anchor after an uncertain match.
            return self.record_unmatched();
        };
        if offset_history_reject_threshold(&self.accepted_offset_history)
            .is_some_and(|threshold| shift > threshold)
        {
            return self.record_unmatched();
        }

        let frame_id = self.next_capture_frame_id();
        let Some(direction) = candidates.selected_direction else {
            return self.record_unmatched();
        };
        let direction = match (axis, direction) {
            (MatchAxis::Vertical, MatchDirection::Forward) => ScrollDirection::Down,
            (MatchAxis::Vertical, MatchDirection::Reverse) => ScrollDirection::Up,
            (MatchAxis::Horizontal, MatchDirection::Forward) => ScrollDirection::Right,
            (MatchAxis::Horizontal, MatchDirection::Reverse) => ScrollDirection::Left,
        };
        self.append_validated_frame(
            current, width, height, shift, axis, direction, frame_id, max_extent,
        )
    }

    fn record_unmatched(&mut self) -> ScrollObservation {
        self.idle_ticks = 0;
        self.unmatched_ticks = self.unmatched_ticks.saturating_add(1);
        ScrollObservation::Idle {
            reached_idle_limit: self.unmatched_ticks >= MAX_CONSECUTIVE_UNMATCHED,
        }
    }
}

fn offset_history_median(history: &[usize]) -> Option<usize> {
    if history.is_empty() {
        return None;
    }
    let mut sorted = history.to_vec();
    sorted.sort_unstable();
    sorted.get(sorted.len() / 2).copied()
}

fn offset_history_reject_threshold(history: &[usize]) -> Option<usize> {
    if history.len() < OFFSET_HISTORY_MIN_SAMPLES {
        return None;
    }
    let median = offset_history_median(history)?;
    Some(median.saturating_mul(4).max(median.saturating_add(120)))
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub(super) enum ScrollObservation {
    Advanced,
    Idle { reached_idle_limit: bool },
    ReachedLimit,
    NoSession,
}

fn grayscale_at(frame: &[u8], pixel_index: usize) -> u8 {
    let offset = pixel_index * 4;
    ((frame[offset] as u16 * 77 + frame[offset + 1] as u16 * 150
        + frame[offset + 2] as u16 * 29)
        >> 8) as u8
}

fn grayscale_feature_map(frame: &[u8], width: usize, height: usize) -> Vec<u8> {
    let pixel_count = width.saturating_mul(height);
    let mut features = vec![0; pixel_count];
    for (index, feature) in features.iter_mut().enumerate() {
        *feature = grayscale_at(frame, index);
    }
    features
}

fn gradient_feature_map(grayscale: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut features = vec![0; width.saturating_mul(height)];
    for y in 0..height {
        for x in 0..width {
            let value = |x, y| grayscale[y * width + x];
            let left = value(x.saturating_sub(1), y);
            let right = value(x.saturating_add(1).min(width - 1), y);
            let top = value(x, y.saturating_sub(1));
            let bottom = value(x, y.saturating_add(1).min(height - 1));
            let horizontal = left.abs_diff(right) / 2;
            let vertical = top.abs_diff(bottom) / 2;
            features[y * width + x] =
                (horizontal as u16 + vertical as u16).div_ceil(2) as u8;
        }
    }
    features
}

fn generate_match_regions(width: usize, use_full_cells: bool) -> Vec<MatchRegion> {
    if width == 0 {
        return Vec::new();
    }
    if width < 64 {
        return vec![MatchRegion {
            x: 0,
            width,
            weight: 1,
        }];
    }

    let region_count = MATCH_REGION_COUNT.min((width / 32).max(1));
    let cell_width = width / region_count;
    let base_region_width = if use_full_cells {
        cell_width
    } else {
        (cell_width / 2).max(16)
    };
    let region_width = base_region_width.min(MATCH_REGION_MAX_WIDTH).min(width);
    let mut regions = Vec::with_capacity(region_count);
    for index in 0..region_count {
        let center = index * cell_width + cell_width / 2;
        let x = center
            .saturating_sub(region_width / 2)
            .min(width.saturating_sub(region_width));
        if regions.iter().any(|region: &MatchRegion| region.x == x) {
            continue;
        }
        regions.push(MatchRegion {
            x,
            width: region_width,
            weight: 1,
        });
    }
    regions
}

fn region_texture_score(
    grayscale: &[u8],
    width: usize,
    height: usize,
    region: MatchRegion,
) -> u32 {
    let mut gradient_sum = 0u64;
    let mut samples = 0u64;
    let region_end = region.x.saturating_add(region.width).min(width);
    let x_stride = (region.width / 32).clamp(1, 8);
    for y in (0..height.saturating_sub(1)).step_by(2) {
        for x in (region.x..region_end).step_by(x_stride) {
            let index = y * width + x;
            gradient_sum += grayscale[index].abs_diff(grayscale[index + width]) as u64;
            samples += 1;
            if x + 1 < region_end {
                gradient_sum += grayscale[index].abs_diff(grayscale[index + 1]) as u64;
                samples += 1;
            }
        }
    }
    if samples == 0 {
        0
    } else {
        (gradient_sum.saturating_mul(100) / samples) as u32
    }
}

fn region_candidate_scores(
    a_grayscale: &[u8],
    b_grayscale: &[u8],
    a_features: &[u8],
    b_features: &[u8],
    width: usize,
    height: usize,
    mut region: MatchRegion,
    min_shift: usize,
    max_shift: usize,
    preferred_shift: Option<usize>,
) -> Result<RegionScores, MatchRejectReason> {
    let texture_score = region_texture_score(a_grayscale, width, height, region)
        .min(region_texture_score(b_grayscale, width, height, region));
    // Narrow text strips often contain useful glyph edges but very little
    // average image energy. Keep those candidates and let cross-region
    // consensus plus the global uniqueness check reject empty/repeated areas.
    let minimum_texture_score = if region.width < 64 { 1 } else { 10 };
    if texture_score < minimum_texture_score {
        return Err(MatchRejectReason::InsufficientStableRegion);
    }
    // Give textured content strips more voting weight while capping the
    // contribution of a single unusually busy region.
    region.weight = (texture_score / 500).clamp(1, 4) as u16;
    let region_end = region.x.saturating_add(region.width).min(width);
    let x_stride = (region.width / 24).clamp(1, 8);
    let y_stride = if region.width < 64 {
        MATCH_SCORE_ROW_STRIDE / 2
    } else {
        MATCH_SCORE_ROW_STRIDE
    };
    let mut scores = Vec::with_capacity(max_shift.saturating_sub(min_shift) + 1);
    for shift in min_shift..=max_shift {
        let overlap = height.saturating_sub(shift);
        let mut error = 0u64;
        let mut samples = 0u64;
        for y in (0..overlap).step_by(y_stride) {
            for x in (region.x..region_end).step_by(x_stride) {
                let left = (y + shift) * width + x;
                let right = y * width + x;
                error += a_features[left].abs_diff(b_features[right]) as u64;
                samples += 1;
            }
        }
        scores.push(if samples == 0 {
            u32::MAX
        } else {
            (error.saturating_mul(100) / samples) as u32
        });
    }

    let (best_index, best_score) = scores
        .iter()
        .copied()
        .enumerate()
        .min_by_key(|(index, score)| {
            (
                *score,
                preferred_shift.map_or(0, |preferred| {
                    preferred.abs_diff(min_shift.saturating_add(*index))
                }),
            )
        })
        .ok_or(MatchRejectReason::NoOverlap)?;
    let best_shift = min_shift + best_index;
    let second_score = scores
        .iter()
        .copied()
        .enumerate()
        .filter(|(index, _)| best_shift.abs_diff(min_shift + *index) >= MATCH_PEAK_SEPARATION)
        .map(|(_, score)| score)
        .min();
    let score_margin = second_score.map(|score| score.saturating_sub(best_score)).unwrap_or(u32::MAX);
    let required_margin = MATCH_MIN_SCORE_MARGIN.max(
        best_score.saturating_mul(MATCH_SCORE_MARGIN_PERCENT) / 100,
    );
    let ambiguous = second_score.is_some() && score_margin < required_margin;

    Ok(RegionScores {
        region,
        best_score,
        ambiguous,
        scores,
    })
}

fn find_vertical_match(
    a: &[u8],
    b: &[u8],
    a_grayscale: &[u8],
    b_grayscale: &[u8],
    a_features: &[u8],
    b_features: &[u8],
    width: usize,
    height: usize,
    use_full_cells: bool,
    direction: MatchDirection,
    motion_hint: Option<MotionSearchHint>,
) -> CandidateMatch {
    let invalid = |reason: MatchRejectReason| CandidateMatch {
        shift: None,
        confidence: 0,
        diagnostics: CandidateDiagnostics::empty(reason),
    };
    if a.len() != b.len()
        || width == 0
        || height == 0
        || a.len() != width.saturating_mul(height).saturating_mul(4)
        || a_grayscale.len() != width.saturating_mul(height)
        || b_grayscale.len() != width.saturating_mul(height)
        || a_features.len() != width.saturating_mul(height)
        || b_features.len() != width.saturating_mul(height)
    {
        return invalid(MatchRejectReason::InvalidFrame);
    }

    let min_overlap = (height / MATCH_MIN_OVERLAP_RATIO)
        .max(8)
        .min(height.saturating_sub(1))
        .min(height.saturating_sub(2.min(height.saturating_sub(1))));
    let full_max_shift = height
        .saturating_sub(min_overlap)
        .min(MATCH_MAX_SHIFT);
    let search_range_capped = height.saturating_sub(min_overlap) > MATCH_MAX_SHIFT;
    let (min_shift, max_shift, preferred_shift, motion_guided) = if let Some(hint) = motion_hint {
        let expected_direction = if hint.signed_offset > 0 {
            MatchDirection::Forward
        } else {
            MatchDirection::Reverse
        };
        if direction != expected_direction {
            return invalid(MatchRejectReason::AxisNotEvaluated);
        }
        let preferred_shift = hint.signed_offset.unsigned_abs() as usize;
        if preferred_shift == 0 || preferred_shift > full_max_shift {
            return invalid(MatchRejectReason::SearchRangeBoundary);
        }
        let min_shift = preferred_shift.saturating_sub(hint.tolerance).max(1);
        let max_shift = preferred_shift
            .saturating_add(hint.tolerance)
            .min(full_max_shift);
        if min_shift > max_shift {
            return invalid(MatchRejectReason::SearchRangeBoundary);
        }
        (min_shift, max_shift, Some(preferred_shift), true)
    } else {
        (1, full_max_shift, None, false)
    };
    let base_diagnostics = CandidateDiagnostics {
        search_min: min_shift as i32,
        search_max: max_shift as i32,
        search_range_capped,
        hit_search_boundary: false,
        best_offset: None,
        best_score: None,
        second_offset: None,
        second_score: None,
        candidate_search_elapsed_ms: 0,
        overlap_extent: 0,
        generated_region_count: 0,
        valid_region_count: 0,
        supporting_region_count: 0,
        ambiguous_region_count: 0,
        low_texture_region_count: 0,
        score_margin: None,
        score_threshold: if motion_guided {
            MOTION_GUIDED_MAX_SCORE
        } else {
            MATCH_MAX_SCORE
        },
        margin_threshold: MATCH_MIN_SCORE_MARGIN,
        valid_regions: Vec::new(),
        supporting_regions: Vec::new(),
        rejection_reason: None,
    };
    if max_shift < min_shift {
        return CandidateMatch {
            diagnostics: CandidateDiagnostics {
                rejection_reason: Some(MatchRejectReason::NoOverlap),
                ..base_diagnostics
            },
            ..invalid(MatchRejectReason::NoOverlap)
        };
    }

    let regions = generate_match_regions(width, use_full_cells);
    let generated_region_count = regions.len() as u32;
    let mut candidates = Vec::new();
    let mut ambiguous_region_count = 0usize;
    let mut low_texture_region_count = 0usize;
    for region in regions {
        match region_candidate_scores(
            a_grayscale,
            b_grayscale,
            a_features,
            b_features,
            width,
            height,
            region,
            min_shift,
            max_shift,
            preferred_shift,
        ) {
            Ok(candidate) => {
                if candidate.ambiguous {
                    ambiguous_region_count += 1;
                }
                candidates.push(candidate);
            }
            Err(MatchRejectReason::InsufficientStableRegion) => low_texture_region_count += 1,
            Err(_) => {}
        }
    }
    if candidates.is_empty() {
        let reason = if ambiguous_region_count > 0 {
            MatchRejectReason::AmbiguousCandidates
        } else {
            MatchRejectReason::InsufficientStableRegion
        };
        return CandidateMatch {
            diagnostics: CandidateDiagnostics {
                generated_region_count,
                valid_region_count: 0,
                ambiguous_region_count: ambiguous_region_count as u32,
                low_texture_region_count: low_texture_region_count as u32,
                rejection_reason: Some(reason),
                ..base_diagnostics
            },
            ..invalid(reason)
        };
    }

    let required_support_count = if motion_guided {
        1usize
    } else if width >= 128 {
        (candidates.len() / 2 + 1).max(2)
    } else {
        1
    };
    let candidate_count = candidates[0].scores.len();
    let mut support_counts = Vec::with_capacity(candidate_count);
    let mut aggregate_scores = Vec::with_capacity(candidate_count);
    for index in 0..candidate_count {
        let mut supporting_weight = 0u64;
        let mut weighted_score = 0u64;
        let mut supporting_count = 0usize;
        for candidate in &candidates {
            let score = candidate.scores[index];
            let local_margin = MATCH_MIN_SCORE_MARGIN.max(
                candidate.best_score.saturating_mul(MATCH_SCORE_MARGIN_PERCENT) / 100,
            );
            if score != u32::MAX
                && score <= candidate.best_score.saturating_add(local_margin)
            {
                supporting_count += 1;
                let weight = candidate.region.weight as u64;
                supporting_weight += weight;
                weighted_score += score as u64 * weight;
            }
        }
        support_counts.push(supporting_count);
        aggregate_scores.push(if supporting_count < required_support_count {
            u32::MAX
        } else {
            (weighted_score / supporting_weight.max(1)) as u32
        });
    }
    let Some((best_index, consensus_score)) = aggregate_scores
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, score)| *score != u32::MAX)
        .min_by_key(|(index, score)| {
            (
                usize::MAX - support_counts[*index],
                *score,
                preferred_shift.map_or(0, |preferred| {
                    preferred.abs_diff(min_shift.saturating_add(*index))
                }),
            )
        })
    else {
        return CandidateMatch {
            diagnostics: CandidateDiagnostics {
                generated_region_count,
                valid_region_count: candidates.len() as u32,
                ambiguous_region_count: ambiguous_region_count as u32,
                low_texture_region_count: low_texture_region_count as u32,
                rejection_reason: Some(MatchRejectReason::InsufficientStableRegion),
                ..base_diagnostics
            },
            ..invalid(MatchRejectReason::InsufficientStableRegion)
        };
    };
    let consensus_shift = min_shift + best_index;
    let consensus_support_count = support_counts[best_index];
    let supporting_regions: Vec<&RegionScores> = candidates
        .iter()
        .filter(|candidate| {
            let score = candidate.scores[best_index];
            let local_margin = MATCH_MIN_SCORE_MARGIN.max(
                candidate.best_score.saturating_mul(MATCH_SCORE_MARGIN_PERCENT) / 100,
            );
            score != u32::MAX && score <= candidate.best_score.saturating_add(local_margin)
        })
        .map(|candidate| candidate)
        .collect();
    let second = aggregate_scores
        .iter()
        .copied()
        .enumerate()
        .filter(|(index, score)| {
            *score != u32::MAX
                && support_counts[*index] == consensus_support_count
                && (min_shift + *index).abs_diff(consensus_shift) >= MATCH_PEAK_SEPARATION
        })
        .map(|(index, score)| (min_shift + index, score))
        .min_by_key(|(_, score)| *score);
    let score_margin = second.map(|(_, score)| score.saturating_sub(consensus_score));
    let required_margin = MATCH_MIN_SCORE_MARGIN.max(
        consensus_score.saturating_mul(MATCH_SCORE_MARGIN_PERCENT) / 100,
    );
    let valid_region_count = candidates.len() as u32;
    let supporting_region_count = supporting_regions.len() as u32;
    let hit_search_boundary = consensus_shift == min_shift || consensus_shift == max_shift;
    let touched_guided_boundary = motion_guided && hit_search_boundary;
    let score_threshold = if motion_guided {
        MOTION_GUIDED_MAX_SCORE
    } else {
        MATCH_MAX_SCORE
    };
    let rejection_reason = if consensus_score > score_threshold {
        Some(MatchRejectReason::ScoreTooLow)
    } else if supporting_region_count < required_support_count as u32 {
        Some(MatchRejectReason::InsufficientStableRegion)
    } else if touched_guided_boundary {
        Some(MatchRejectReason::SearchRangeBoundary)
    } else if !motion_guided && score_margin.is_some_and(|margin| margin < required_margin) {
        Some(MatchRejectReason::AmbiguousCandidates)
    } else if search_range_capped && consensus_shift == max_shift {
        Some(MatchRejectReason::SearchRangeBoundary)
    } else {
        None
    };
    let diagnostics = CandidateDiagnostics {
        search_min: min_shift as i32,
        search_max: max_shift as i32,
        search_range_capped,
        hit_search_boundary,
        best_offset: Some(consensus_shift as i32),
        best_score: Some(consensus_score),
        second_offset: second.map(|(offset, _)| offset as i32),
        second_score: second.map(|(_, score)| score),
        candidate_search_elapsed_ms: 0,
        overlap_extent: height.saturating_sub(consensus_shift) as u32,
        generated_region_count,
        valid_region_count,
        supporting_region_count,
        ambiguous_region_count: ambiguous_region_count as u32,
        low_texture_region_count: low_texture_region_count as u32,
        score_margin,
        score_threshold,
        margin_threshold: required_margin,
        valid_regions: candidates.iter().map(|candidate| candidate.region).collect(),
        supporting_regions: supporting_regions
            .iter()
            .map(|candidate| candidate.region)
            .collect(),
        rejection_reason,
    };
    let confidence = (25_500u32.saturating_sub(consensus_score) * 100 / 25_500) as u16;
    CandidateMatch {
        shift: rejection_reason.is_none().then_some(consensus_shift),
        confidence,
        diagnostics,
    }
}

fn select_match_direction(
    forward: &CandidateMatch,
    reverse: &CandidateMatch,
) -> (Option<MatchDirection>, Option<usize>, Option<MatchRejectReason>) {
    match (forward.shift, reverse.shift) {
        (Some(forward_shift), None) => {
            (Some(MatchDirection::Forward), Some(forward_shift), None)
        }
        (None, Some(reverse_shift)) => (
            Some(MatchDirection::Reverse),
            Some(reverse_shift),
            None,
        ),
        (Some(forward_shift), Some(reverse_shift)) => {
            let forward_score = forward.diagnostics.best_score.unwrap_or(u32::MAX);
            let reverse_score = reverse.diagnostics.best_score.unwrap_or(u32::MAX);
            let forward_regions = forward.diagnostics.supporting_region_count;
            let reverse_regions = reverse.diagnostics.supporting_region_count;
            if forward_regions >= reverse_regions.saturating_add(2)
                || forward_score.saturating_add(MATCH_DIRECTION_SCORE_MARGIN) < reverse_score
            {
                (Some(MatchDirection::Forward), Some(forward_shift), None)
            } else if reverse_regions >= forward_regions.saturating_add(2)
                || reverse_score.saturating_add(MATCH_DIRECTION_SCORE_MARGIN) < forward_score
            {
                (
                    Some(MatchDirection::Reverse),
                    Some(reverse_shift),
                    None,
                )
            } else {
                (None, None, Some(MatchRejectReason::AmbiguousDirection))
            }
        }
        (None, None) => {
            let reasons = [
                forward.diagnostics.rejection_reason,
                reverse.diagnostics.rejection_reason,
            ];
            let reason = [
                MatchRejectReason::SearchRangeBoundary,
                MatchRejectReason::AmbiguousCandidates,
                MatchRejectReason::InsufficientStableRegion,
                MatchRejectReason::ScoreTooLow,
            ]
            .into_iter()
            .find(|candidate| reasons.contains(&Some(*candidate)))
            .unwrap_or(MatchRejectReason::NoOverlap);
            (None, None, Some(reason))
        }
    }
}

fn transpose_rgba(frame: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut transposed = vec![0; frame.len()];
    for y in 0..height {
        for x in 0..width {
            let source = (y * width + x) * 4;
            let target = (x * height + y) * 4;
            transposed[target..target + 4].copy_from_slice(&frame[source..source + 4]);
        }
    }
    transposed
}

fn find_axis_candidates(
    previous: &[u8],
    current: &[u8],
    width: usize,
    height: usize,
    axis: MatchAxis,
    motion_hint: Option<MotionSearchHint>,
) -> AxisCandidates {
    use std::time::Instant;

    let previous_transposed = (axis == MatchAxis::Horizontal)
        .then(|| transpose_rgba(previous, width, height));
    let current_transposed = (axis == MatchAxis::Horizontal)
        .then(|| transpose_rgba(current, width, height));
    let (previous, current, match_width, match_height) = match axis {
        MatchAxis::Vertical => (previous, current, width, height),
        MatchAxis::Horizontal => (
            previous_transposed.as_deref().unwrap_or_default(),
            current_transposed.as_deref().unwrap_or_default(),
            height,
            width,
        ),
    };
    let use_full_cells = axis == MatchAxis::Horizontal || width <= 256;
    let grayscale_preprocess_started = Instant::now();
    let previous_grayscale = grayscale_feature_map(previous, match_width, match_height);
    let current_grayscale = grayscale_feature_map(current, match_width, match_height);
    let grayscale_preprocess_elapsed_ms = grayscale_preprocess_started.elapsed().as_millis();
    let mut gradient_features = None;

    if let Some(hint) = motion_hint {
        let mut guided = find_axis_candidates_with_maps(
            previous,
            current,
            &previous_grayscale,
            &current_grayscale,
            match_width,
            match_height,
            use_full_cells,
            grayscale_preprocess_elapsed_ms,
            &mut gradient_features,
            Some(hint),
        );
        if guided.shift.is_some() {
            guided.motion_guided = true;
            guided.motion_calibration_eligible = false;
            return guided;
        }
        let fallback_reason = guided.rejection_reason.unwrap_or(MatchRejectReason::NoOverlap);
        let mut recovered = find_axis_candidates_with_maps(
            previous,
            current,
            &previous_grayscale,
            &current_grayscale,
            match_width,
            match_height,
            use_full_cells,
            grayscale_preprocess_elapsed_ms,
            &mut gradient_features,
            None,
        );
        recovered.motion_guided_fallback_reason = Some(fallback_reason);
        return recovered;
    }

    find_axis_candidates_with_maps(
        previous,
        current,
        &previous_grayscale,
        &current_grayscale,
        match_width,
        match_height,
        use_full_cells,
        grayscale_preprocess_elapsed_ms,
        &mut gradient_features,
        None,
    )
}

fn find_axis_candidates_with_maps(
    previous: &[u8],
    current: &[u8],
    previous_grayscale: &[u8],
    current_grayscale: &[u8],
    match_width: usize,
    match_height: usize,
    use_full_cells: bool,
    grayscale_preprocess_elapsed_ms: u128,
    gradient_features: &mut Option<(Vec<u8>, Vec<u8>, u128)>,
    motion_hint: Option<MotionSearchHint>,
) -> AxisCandidates {
    use std::time::Instant;
    let forward_started = Instant::now();
    let mut forward = find_vertical_match(
        previous,
        current,
        &previous_grayscale,
        &current_grayscale,
        &previous_grayscale,
        &current_grayscale,
        match_width,
        match_height,
        use_full_cells,
        MatchDirection::Forward,
        motion_hint,
    );
    forward.diagnostics.candidate_search_elapsed_ms = forward_started.elapsed().as_millis();
    let reverse_started = Instant::now();
    let mut reverse = find_vertical_match(
        current,
        previous,
        &current_grayscale,
        &previous_grayscale,
        &current_grayscale,
        &previous_grayscale,
        match_width,
        match_height,
        use_full_cells,
        MatchDirection::Reverse,
        motion_hint,
    );
    reverse.diagnostics.candidate_search_elapsed_ms = reverse_started.elapsed().as_millis();
    let (mut selected_direction, mut shift, mut rejection_reason) =
        select_match_direction(&forward, &reverse);
    let mut selected_metric = MatchMetric::Grayscale;
    let mut gradient_forward = None;
    let mut gradient_reverse = None;
    let mut gradient_preprocess_elapsed_ms = gradient_features
        .as_ref()
        .map_or(0, |(_, _, elapsed_ms)| *elapsed_ms);

    if shift.is_none() || motion_hint.is_none() {
        if gradient_features.is_none() {
            let preprocess_started = Instant::now();
            *gradient_features = Some((
                gradient_feature_map(previous_grayscale, match_width, match_height),
                gradient_feature_map(current_grayscale, match_width, match_height),
                preprocess_started.elapsed().as_millis(),
            ));
        }
        let (gradient_previous, gradient_current, elapsed_ms) = gradient_features
            .as_ref()
            .expect("gradient maps were prepared for fallback search");
        gradient_preprocess_elapsed_ms = *elapsed_ms;
        let gradient_forward_started = Instant::now();
        let mut forward_candidate = find_vertical_match(
            previous,
            current,
            &previous_grayscale,
            &current_grayscale,
            gradient_previous,
            gradient_current,
            match_width,
            match_height,
            use_full_cells,
            MatchDirection::Forward,
            motion_hint,
        );
        forward_candidate.diagnostics.candidate_search_elapsed_ms =
            gradient_forward_started.elapsed().as_millis();
        let gradient_reverse_started = Instant::now();
        let mut reverse_candidate = find_vertical_match(
            current,
            previous,
            &current_grayscale,
            &previous_grayscale,
            gradient_current,
            gradient_previous,
            match_width,
            match_height,
            use_full_cells,
            MatchDirection::Reverse,
            motion_hint,
        );
        reverse_candidate.diagnostics.candidate_search_elapsed_ms =
            gradient_reverse_started.elapsed().as_millis();
        let (gradient_direction, gradient_shift, gradient_rejection) =
            select_match_direction(&forward_candidate, &reverse_candidate);
        if let Some(gradient_shift) = gradient_shift {
            if let Some(grayscale_shift) = shift {
                let agrees = selected_direction == gradient_direction
                    && grayscale_shift.abs_diff(gradient_shift)
                        <= MATCHER_AGREEMENT_TOLERANCE;
                if agrees {
                    selected_metric = MatchMetric::CrossValidated;
                } else {
                    selected_direction = None;
                    shift = None;
                    rejection_reason = Some(MatchRejectReason::MatcherDisagreement);
                }
            } else {
                selected_direction = gradient_direction;
                shift = Some(gradient_shift);
                rejection_reason = gradient_rejection;
                selected_metric = MatchMetric::Gradient;
            }
        }
        gradient_forward = Some(forward_candidate);
        gradient_reverse = Some(reverse_candidate);
    }

    let confidence = match (selected_metric, selected_direction) {
        (MatchMetric::Grayscale, Some(MatchDirection::Forward)) => forward.confidence,
        (MatchMetric::Grayscale, Some(MatchDirection::Reverse)) => reverse.confidence,
        (MatchMetric::Gradient, Some(MatchDirection::Forward)) => {
            gradient_forward.as_ref().map_or(0, |candidate| candidate.confidence)
        }
        (MatchMetric::Gradient, Some(MatchDirection::Reverse)) => {
            gradient_reverse.as_ref().map_or(0, |candidate| candidate.confidence)
        }
        (MatchMetric::CrossValidated, Some(MatchDirection::Forward)) => forward
            .confidence
            .max(gradient_forward.as_ref().map_or(0, |candidate| candidate.confidence))
            .saturating_add(2)
            .min(100),
        (MatchMetric::CrossValidated, Some(MatchDirection::Reverse)) => reverse
            .confidence
            .max(gradient_reverse.as_ref().map_or(0, |candidate| candidate.confidence))
            .saturating_add(2)
            .min(100),
        (_, None) => 0,
    };
    let selected_candidate = match (selected_metric, selected_direction) {
        (MatchMetric::Grayscale, Some(MatchDirection::Forward)) => Some(&forward),
        (MatchMetric::Grayscale, Some(MatchDirection::Reverse)) => Some(&reverse),
        (MatchMetric::Gradient, Some(MatchDirection::Forward)) => gradient_forward.as_ref(),
        (MatchMetric::Gradient, Some(MatchDirection::Reverse)) => gradient_reverse.as_ref(),
        (MatchMetric::CrossValidated, Some(MatchDirection::Forward)) => Some(&forward),
        (MatchMetric::CrossValidated, Some(MatchDirection::Reverse)) => Some(&reverse),
        (_, None) => None,
    };
    let motion_calibration_eligible = motion_hint.is_none()
        && selected_candidate.is_some_and(|candidate| {
            candidate.diagnostics.supporting_region_count >= 2
                && candidate
                    .diagnostics
                    .best_score
                    .is_some_and(|score| score <= 1200)
                && candidate
                    .diagnostics
                    .score_margin
                    .is_some_and(|margin| margin >= candidate.diagnostics.margin_threshold)
        });
    AxisCandidates {
        forward,
        reverse,
        gradient_forward,
        gradient_reverse,
        selected_direction,
        selected_metric,
        shift,
        confidence,
        motion_guided: motion_hint.is_some() && shift.is_some(),
        motion_guided_fallback_reason: None,
        motion_calibration_eligible,
        grayscale_preprocess_elapsed_ms,
        gradient_preprocess_elapsed_ms,
        rejection_reason,
    }
}

pub(super) fn estimate_vertical_shift(
    a: &[u8],
    b: &[u8],
    width: usize,
    height: usize,
) -> Option<usize> {
    if a.len() != b.len() || a.len() != width.saturating_mul(height).saturating_mul(4) {
        return None;
    }
    let a_grayscale = grayscale_feature_map(a, width, height);
    let b_grayscale = grayscale_feature_map(b, width, height);
    find_vertical_match(
        a,
        b,
        &a_grayscale,
        &b_grayscale,
        &a_grayscale,
        &b_grayscale,
        width,
        height,
        false,
        MatchDirection::Forward,
        None,
    )
    .shift
}

#[cfg(test)]
mod tests {
    use super::estimate_vertical_shift;

    fn frame(width: usize, height: usize, offset: u8) -> Vec<u8> {
        (0..height)
            .flat_map(|y| (0..width).flat_map(move |_| [offset.wrapping_add(y as u8), 0, 0, 255]))
            .collect()
    }

    #[test]
    fn estimates_vertical_shift_in_scroll_direction() {
        let width = 2;
        let height = 12;
        let mut a = vec![0; width * height * 4];
        let mut b = vec![0; width * height * 4];
        for y in 0..height {
            for x in 0..width {
                let value = ((y + 10) % 255) as u8;
                a[(y * width + x) * 4] = value;
                b[(y * width + x) * 4] = if y + 2 < height {
                    ((y + 12) % 255) as u8
                } else {
                    0
                };
            }
        }
        assert_eq!(estimate_vertical_shift(&a, &b, width, height), Some(2));
    }

    #[test]
    fn accepts_a_one_pixel_scroll() {
        let width = 2;
        let height = 12;
        let a = frame(width, height, 0);
        let mut b = frame(width, height, 1);
        for pixel in b[11 * width * 4..].as_chunks_mut::<4>().0 {
            pixel[0] = 0;
        }
        assert_eq!(estimate_vertical_shift(&a, &b, width, height), Some(1));
    }

    #[test]
    fn repeated_frame_is_idle() {
        let width = 1;
        let height = 8;
        let frame = [12, 0, 0, 255].repeat(width * height);
        let mut capture = super::ScrollingCapture::pending();
        capture.seed(width, height, frame.clone());
        let result = capture.observe(frame, width, height, 100);
        assert!(matches!(
            result,
            super::ScrollObservation::Idle {
                reached_idle_limit: false
            }
        ));
        assert_eq!(capture.assembled.as_ref().unwrap().1, height);
    }

    #[test]
    fn unmatched_frame_does_not_move_baseline() {
        let width = 1;
        let height = 12;
        let mut capture = super::ScrollingCapture::pending();
        capture.seed(width, height, frame(width, height, 0));
        let bad = [200, 0, 0, 255].repeat(width * height);
        assert!(matches!(
            capture.observe(bad, width, height, 100),
            super::ScrollObservation::Idle {
                reached_idle_limit: false
            }
        ));
        assert!(matches!(
            capture.observe(frame(width, height, 2), width, height, 100),
            super::ScrollObservation::Advanced
        ));
        assert_eq!(capture.assembled.as_ref().unwrap().1, 14);
    }

    #[test]
    fn repeated_unmatched_frames_stop_after_retry_budget() {
        let width = 2;
        let height = 8;
        let mut capture = super::ScrollingCapture::pending();
        capture.seed(width, height, frame(width, height, 0));
        for attempt in 0..10 {
            let result = capture.observe(vec![1, 2, 3, 4], width, height, 100);
            if attempt < 9 {
                assert!(matches!(
                    result,
                    super::ScrollObservation::Idle {
                        reached_idle_limit: false
                    }
                ));
            } else {
                assert!(matches!(
                    result,
                    super::ScrollObservation::Idle {
                        reached_idle_limit: true
                    }
                ));
            }
        }
    }

    #[test]
    fn observer_requires_settling_before_capture() {
        let mut capture = super::ScrollingCapture::pending();
        capture.seed_probe(super::FrameProbe::new(vec![0; 100]));
        assert_eq!(
            capture.observe_probe(super::FrameProbe::new(vec![40; 100])),
            super::FrameResult::MotionDetected
        );
        assert!(!capture.ready_to_capture());
        capture.observe_probe(super::FrameProbe::new(vec![40; 100]));
        assert!(!capture.ready_to_capture());
        capture.observe_probe(super::FrameProbe::new(vec![40; 100]));
        assert!(capture.ready_to_capture());
    }

    #[test]
    fn new_motion_resets_settling_debounce() {
        let mut capture = super::ScrollingCapture::pending();
        capture.seed_probe(super::FrameProbe::new(vec![0; 8]));
        assert_eq!(
            capture.observe_probe(super::FrameProbe::new(vec![40; 8])),
            super::FrameResult::MotionDetected
        );
        assert_eq!(
            capture.observe_probe(super::FrameProbe::new(vec![40; 8])),
            super::FrameResult::Unchanged
        );
        assert_eq!(
            capture.observe_probe(super::FrameProbe::new(vec![80; 8])),
            super::FrameResult::MotionDetected
        );
        assert!(!capture.ready_to_capture());
        capture.observe_probe(super::FrameProbe::new(vec![80; 8]));
        assert!(!capture.ready_to_capture());
        capture.observe_probe(super::FrameProbe::new(vec![80; 8]));
        assert!(capture.ready_to_capture());
    }

    #[test]
    fn stable_observation_sequence_appends_once_then_returns_to_observing() {
        let (width, height) = (1, 8);
        let initial = frame(width, height, 0);
        let shifted = frame(width, height, 2);
        let mut capture = super::ScrollingCapture::pending();
        capture.seed(width, height, initial);
        capture.seed_probe(super::FrameProbe::new(vec![0; 100]));

        assert_eq!(
            capture.observe_probe(super::FrameProbe::new(vec![40; 100])),
            super::FrameResult::MotionDetected
        );
        assert!(!capture.ready_to_capture());
        assert_eq!(
            capture.observe_probe(super::FrameProbe::new(vec![40; 100])),
            super::FrameResult::Unchanged
        );
        assert!(!capture.ready_to_capture());
        assert_eq!(
            capture.observe_probe(super::FrameProbe::new(vec![40; 100])),
            super::FrameResult::Unchanged
        );
        assert!(capture.ready_to_capture());

        assert!(matches!(
            capture.capture_stable(shifted, width, height, 100),
            super::FrameResult::Matched { shift: 2, .. }
        ));
        assert_eq!(capture.phase, super::CapturePhase::Observing);
        assert_eq!(capture.assembled.as_ref().unwrap().1, height + 2);

        assert_eq!(
            capture.observe_probe(super::FrameProbe::new(vec![40; 100])),
            super::FrameResult::Unchanged
        );
        assert!(!capture.ready_to_capture());
        assert_eq!(capture.assembled.as_ref().unwrap().1, height + 2);
    }
    #[test]
    fn probe_change_threshold_requires_eight_samples() {
        let probe_len = super::SCROLL_PROBE_GRID * super::SCROLL_PROBE_GRID;
        let mut below_capture = super::ScrollingCapture::pending();
        below_capture.seed_probe(super::FrameProbe::new(vec![0; probe_len]));
        let mut below = vec![0; probe_len];
        below[..7].fill(40);
        assert_eq!(
            below_capture.observe_probe(super::FrameProbe::new(below)),
            super::FrameResult::Unchanged
        );

        let mut threshold_capture = super::ScrollingCapture::pending();
        threshold_capture.seed_probe(super::FrameProbe::new(vec![0; probe_len]));
        let mut at_threshold = vec![0; probe_len];
        at_threshold[..8].fill(40);
        assert_eq!(
            threshold_capture.observe_probe(super::FrameProbe::new(at_threshold)),
            super::FrameResult::MotionDetected
        );
    }
    #[test]
    fn small_probe_changes_accumulate_against_stable_baseline() {
        let mut capture = super::ScrollingCapture::pending();
        capture.seed_probe(super::FrameProbe::new(vec![0; 100]));
        for changed in 1..8 {
            let mut probe = vec![0; 100];
            probe[..changed].fill(40);
            assert_eq!(
                capture.observe_probe(super::FrameProbe::new(probe)),
                super::FrameResult::Unchanged
            );
        }
        let mut threshold_probe = vec![0; 100];
        threshold_probe[..8].fill(40);
        assert_eq!(
            capture.observe_probe(super::FrameProbe::new(threshold_probe)),
            super::FrameResult::MotionDetected
        );
    }
    #[test]
    fn invalid_probe_pauses_without_ending_session() {
        let mut capture = super::ScrollingCapture::pending();
        capture.seed_probe(super::FrameProbe::new(vec![0; 8]));
        assert_eq!(
            capture.observe_probe(super::FrameProbe::new(vec![1; 7])),
            super::FrameResult::InvalidFrame
        );
        assert_eq!(capture.phase, super::CapturePhase::Paused);
    }

    #[test]
    fn empty_probe_pauses_without_capture() {
        let mut capture = super::ScrollingCapture::pending();
        capture.seed_probe(super::FrameProbe::new(vec![0; 8]));

        assert_eq!(
            capture.observe_probe(super::FrameProbe::new(Vec::new())),
            super::FrameResult::InvalidFrame
        );
        assert_eq!(capture.phase, super::CapturePhase::Paused);
        assert!(!capture.ready_to_capture());
    }

    #[test]
    fn paused_session_ignores_static_probe_until_new_motion() {
        let mut capture = super::ScrollingCapture::pending();
        capture.seed_probe(super::FrameProbe::new(vec![0; 8]));
        capture.phase = super::CapturePhase::Paused;

        assert_eq!(
            capture.observe_probe(super::FrameProbe::new(vec![0; 8])),
            super::FrameResult::Unchanged
        );
        assert_eq!(capture.phase, super::CapturePhase::Paused);
        assert!(!capture.ready_to_capture());
    }

    #[test]
    fn no_overlap_does_not_close_session() {
        let (width, height) = (1, 12);
        let mut capture = super::ScrollingCapture::pending();
        capture.seed(width, height, frame(width, height, 0));
        capture.phase = super::CapturePhase::Capturing;
        let result =
            capture.capture_stable([200, 1, 2, 255].repeat(width * height), width, height, 100);
        assert!(matches!(
            result,
            super::FrameResult::NoOverlap | super::FrameResult::ReverseDirection { .. }
        ));
        assert!(capture.assembled.is_some());
        assert_eq!(capture.phase, super::CapturePhase::Paused);
    }

    #[test]
    fn no_overlap_pause_can_resume_on_later_valid_frame() {
        let (width, height) = (1, 12);
        let initial = frame(width, height, 0);
        let mut capture = super::ScrollingCapture::pending();
        capture.seed(width, height, initial);
        capture.seed_probe(super::FrameProbe::new(vec![0; 8]));
        capture.phase = super::CapturePhase::Capturing;
        let bad = [200, 1, 2, 255].repeat(width * height);
        let result = capture.capture_stable(bad, width, height, 100);
        assert!(matches!(
            result,
            super::FrameResult::NoOverlap | super::FrameResult::ReverseDirection { .. }
        ));
        assert_eq!(capture.phase, super::CapturePhase::Paused);
        assert_eq!(capture.assembled.as_ref().unwrap().1, height);
        assert_eq!(
            capture.observe_probe(super::FrameProbe::new(vec![40; 8])),
            super::FrameResult::MotionDetected
        );
        capture.observe_probe(super::FrameProbe::new(vec![40; 8]));
        capture.observe_probe(super::FrameProbe::new(vec![40; 8]));
        assert!(capture.ready_to_capture());
        assert!(matches!(
            capture.capture_stable(frame(width, height, 2), width, height, 100),
            super::FrameResult::Matched { .. }
        ));
        assert_eq!(capture.phase, super::CapturePhase::Observing);
        assert_eq!(capture.assembled.as_ref().unwrap().1, height + 2);
    }

    #[test]
    fn fast_scroll_beyond_matcher_window_pauses_without_appending() {
        let (width, height, shift) = (1usize, 2401usize, 1501usize);
        let pixel = |row: usize| {
            let mut hash = (row as u32).wrapping_add(0x9e37_79b9);
            hash = (hash ^ (hash >> 16)).wrapping_mul(0x85eb_ca6b);
            hash = (hash ^ (hash >> 13)).wrapping_mul(0xc2b2_ae35);
            hash ^= hash >> 16;
            [hash as u8, (hash >> 8) as u8, (hash >> 16) as u8, 255]
        };
        let mut initial = Vec::with_capacity(width * height * 4);
        let mut fast = Vec::with_capacity(width * height * 4);
        for row in 0..height {
            initial.extend_from_slice(&pixel(row));
            fast.extend_from_slice(&pixel(row + shift));
        }

        let mut capture = super::ScrollingCapture::pending();
        capture.seed(width, height, initial);
        capture.phase = super::CapturePhase::Capturing;
        let result = capture.capture_stable(fast, width, height, height * 2);

        assert_eq!(result, super::FrameResult::NoOverlap);
        assert_eq!(capture.phase, super::CapturePhase::Paused);
        assert_eq!(capture.assembled.as_ref().unwrap().1, height);
    }

    #[test]
    fn reverse_scroll_is_paused_without_appending() {
        let (width, height) = (1, 12);
        let previous = frame(width, height, 10);
        let mut reverse = vec![0u8; width * height * 4];
        for y in 0..height {
            let value = if y < 2 {
                220 + y as u8
            } else {
                10 + (y - 2) as u8
            };
            reverse[y * 4..y * 4 + 4].copy_from_slice(&[value, 0, 0, 255]);
        }
        let mut capture = super::ScrollingCapture::pending();
        capture.seed(width, height, previous);
        capture.phase = super::CapturePhase::Capturing;
        let result = capture.capture_stable(reverse, width, height, 100);
        assert!(
            matches!(result, super::FrameResult::ReverseDirection { shift: 2, confidence } if confidence > 0)
        );
        assert_eq!(capture.phase, super::CapturePhase::Paused);
        assert_eq!(capture.assembled.as_ref().unwrap().1, height);
    }

    #[test]
    fn reverse_scroll_after_progress_does_not_append() {
        let (width, height) = (1, 12);
        let initial = frame(width, height, 10);
        let forward = frame(width, height, 12);
        let mut capture = super::ScrollingCapture::pending();
        capture.seed(width, height, initial);
        capture.phase = super::CapturePhase::Capturing;

        assert!(matches!(
            capture.capture_stable(forward, width, height, 100),
            super::FrameResult::Matched { shift: 2, .. }
        ));
        let height_after_forward = capture.assembled.as_ref().unwrap().1;
        assert_eq!(capture.direction, Some(super::ScrollDirection::Down));

        // The next frame moves back toward the already captured content. It
        // must pause rather than append rows in the opposite direction.
        let mut reverse = vec![0u8; width * height * 4];
        for y in 0..height {
            let value = if y < 2 {
                8 + y as u8
            } else {
                10 + (y - 2) as u8
            };
            reverse[y * 4..y * 4 + 4].copy_from_slice(&[value, 0, 0, 255]);
        }
        capture.phase = super::CapturePhase::Capturing;
        let result = capture.capture_stable(reverse, width, height, 100);
        assert!(matches!(
            result,
            super::FrameResult::ReverseDirection { shift: 4, confidence } if confidence > 0
        ));
        assert_eq!(capture.phase, super::CapturePhase::Paused);
        assert_eq!(capture.assembled.as_ref().unwrap().1, height_after_forward);
    }

    #[test]
    fn forward_scroll_after_progress_remains_forward() {
        let (width, height) = (1, 12);
        let mut capture = super::ScrollingCapture::pending();
        capture.seed(width, height, frame(width, height, 10));
        capture.phase = super::CapturePhase::Capturing;

        assert!(matches!(
            capture.capture_stable(frame(width, height, 12), width, height, 100),
            super::FrameResult::Matched { shift: 2, .. }
        ));
        let height_after_first = capture.assembled.as_ref().unwrap().1;

        capture.phase = super::CapturePhase::Capturing;
        assert!(matches!(
            capture.capture_stable(frame(width, height, 14), width, height, 100),
            super::FrameResult::Matched { shift: 2, .. }
        ));
        assert_eq!(
            capture.assembled.as_ref().unwrap().1,
            height_after_first + 2
        );
        assert_eq!(capture.phase, super::CapturePhase::Observing);
    }

    #[test]
    fn first_valid_match_sets_forward_direction_and_confidence() {
        let mut capture = super::ScrollingCapture::pending();
        capture.seed(1, 8, frame(1, 8, 0));
        capture.phase = super::CapturePhase::Capturing;
        let result = capture.capture_stable(frame(1, 8, 2), 1, 8, 100);
        assert!(matches!(result, super::FrameResult::Matched { confidence, .. } if confidence > 0));
        assert_eq!(capture.direction, Some(super::ScrollDirection::Down));
        assert_eq!(capture.phase, super::CapturePhase::Observing);
    }

    #[test]
    fn missing_seed_frame_pauses_after_stable_capture() {
        let mut capture = super::ScrollingCapture::pending();
        capture.phase = super::CapturePhase::Capturing;
        let result = capture.capture_stable(vec![0; 8 * 4], 1, 8, 100);
        assert_eq!(result, super::FrameResult::InvalidFrame);
        assert_eq!(capture.phase, super::CapturePhase::Paused);
    }

    #[test]
    fn external_capture_failure_pauses_and_commits_latest_probe() {
        let mut capture = super::ScrollingCapture::pending();
        capture.seed_probe(super::FrameProbe::new(vec![0; 8]));
        assert_eq!(
            capture.observe_probe(super::FrameProbe::new(vec![40; 8])),
            super::FrameResult::MotionDetected
        );
        capture.pause_after_capture_failure();
        assert_eq!(capture.phase, super::CapturePhase::Paused);
        assert!(!capture.ready_to_capture());
        assert_eq!(capture.probe, Some(super::FrameProbe::new(vec![40; 8])));
    }

    #[test]
    fn paused_capture_failure_resumes_on_new_motion() {
        let (width, height) = (1, 8);
        let mut capture = super::ScrollingCapture::pending();
        capture.seed(width, height, frame(width, height, 0));
        capture.seed_probe(super::FrameProbe::new(vec![0; 8]));
        assert_eq!(
            capture.observe_probe(super::FrameProbe::new(vec![40; 8])),
            super::FrameResult::MotionDetected
        );
        capture.pause_after_capture_failure();
        assert_eq!(capture.phase, super::CapturePhase::Paused);

        assert_eq!(
            capture.observe_probe(super::FrameProbe::new(vec![80; 8])),
            super::FrameResult::MotionDetected
        );
        capture.observe_probe(super::FrameProbe::new(vec![80; 8]));
        capture.observe_probe(super::FrameProbe::new(vec![80; 8]));
        assert!(capture.ready_to_capture());
        assert!(matches!(
            capture.capture_stable(frame(width, height, 2), width, height, 100),
            super::FrameResult::Matched { .. }
        ));
        assert_eq!(capture.phase, super::CapturePhase::Observing);
    }

    #[test]
    fn invalid_session_pauses_after_stable_capture() {
        let mut capture = super::ScrollingCapture::pending();
        capture.seed(1, 8, frame(1, 8, 0));
        capture.assembled.as_mut().unwrap().0 = 2;
        capture.phase = super::CapturePhase::Capturing;
        let result = capture.capture_stable(frame(1, 8, 2), 1, 8, 100);
        assert_eq!(result, super::FrameResult::InvalidFrame);
        assert_eq!(capture.phase, super::CapturePhase::Paused);
    }
    #[test]
    fn malformed_stable_frame_pauses_without_mutating_assembled() {
        let mut capture = super::ScrollingCapture::pending();
        capture.seed(2, 8, frame(2, 8, 0));
        let before = capture.assembled.clone();
        capture.phase = super::CapturePhase::Capturing;

        let result = capture.capture_stable(vec![0; 3], 2, 8, 100);

        assert_eq!(result, super::FrameResult::InvalidFrame);
        assert_eq!(capture.phase, super::CapturePhase::Paused);
        assert_eq!(capture.assembled, before);
    }

    #[test]
    fn reached_limit_pauses_after_stable_capture() {
        let mut capture = super::ScrollingCapture::pending();
        capture.seed(1, 8, frame(1, 8, 0));
        capture.phase = super::CapturePhase::Capturing;
        let result = capture.capture_stable(frame(1, 8, 2), 1, 8, 8);
        assert_eq!(result, super::FrameResult::ReachedLimit);
        assert_eq!(capture.phase, super::CapturePhase::Paused);
    }

    #[test]
    fn stable_capture_caps_partial_append_without_overshoot() {
        let width = 1;
        let height = 8;
        let mut capture = super::ScrollingCapture::pending();
        capture.seed(width, height, frame(width, height, 0));
        capture.phase = super::CapturePhase::Capturing;

        let result = capture.capture_stable(frame(width, height, 2), width, height, 9);

        assert_eq!(result, super::FrameResult::ReachedLimit);
        assert_eq!(capture.phase, super::CapturePhase::Paused);
        let (_, assembled_height, pixels) = capture.assembled.unwrap();
        assert_eq!(assembled_height, 9);
        assert_eq!(pixels.len(), width * assembled_height * 4);
    }

    #[test]
    fn max_height_caps_partial_append_without_overshoot() {
        let width = 1;
        let height = 8;
        let mut capture = super::ScrollingCapture::pending();
        capture.seed(width, height, frame(width, height, 0));
        let result = capture.observe(frame(width, height, 2), width, height, 9);
        assert!(matches!(result, super::ScrollObservation::ReachedLimit));
        let (_, assembled_height, pixels) = capture.assembled.unwrap();
        assert_eq!(assembled_height, 9);
        assert_eq!(pixels.len(), width * assembled_height * 4);
    }

    #[test]
    fn appends_only_new_rows() {
        let width = 1;
        let height = 8;
        let frame = |offset: u8| {
            (0..height)
                .flat_map(|y| [offset + y as u8, 0, 0, 255])
                .collect::<Vec<_>>()
        };
        let mut capture = super::ScrollingCapture::pending();
        capture.seed(width, height, frame(0));
        let result = capture.observe(frame(2), width, height, 100);
        assert!(matches!(result, super::ScrollObservation::Advanced));
        let (_, assembled_height, pixels) = capture.assembled.unwrap();
        assert_eq!(assembled_height, 10);
        assert_eq!(pixels[(8 * 4)..(10 * 4)], [8, 0, 0, 255, 9, 0, 0, 255]);
    }

    #[test]
    #[ignore = "synthetic long-scroll benchmark"]
    fn synthetic_long_scroll_benchmark() {
        use std::time::Instant;

        let width = 640usize;
        let height = 360usize;
        let step = 90usize;
        let frames = 48usize;
        let frame_at = |offset: usize| {
            let mut pixels = Vec::with_capacity(width * height * 4);
            for y in 0..height {
                for x in 0..width {
                    let row = offset + y;
                    let hash = row
                        .wrapping_mul(1_103_515_245)
                        .wrapping_add(x.wrapping_mul(12_345)) as u8;
                    pixels.extend_from_slice(&[
                        hash,
                        hash.rotate_left(1),
                        hash.rotate_left(2),
                        255,
                    ]);
                }
            }
            pixels
        };

        let mut capture = super::ScrollingCapture::pending();
        capture.seed(width, height, frame_at(0));
        let started = Instant::now();
        for index in 1..frames {
            let result = capture.observe(frame_at(index * step), width, height, 60_000);
            assert!(
                matches!(result, super::ScrollObservation::Advanced),
                "synthetic frame {index} did not advance"
            );
        }
        let elapsed = started.elapsed();
        let (assembled_width, assembled_height, pixels) = capture.assembled.as_ref().unwrap();
        let expected_height = height + (frames - 1) * step;
        println!(
            "synthetic long-scroll: {} frames, {}x{}, {} ms, {} bytes",
            frames,
            assembled_width,
            assembled_height,
            elapsed.as_millis(),
            pixels.len()
        );
        assert_eq!(*assembled_width, width);
        assert_eq!(*assembled_height, expected_height);
        assert_eq!(pixels.len(), width * expected_height * 4);
    }
}
