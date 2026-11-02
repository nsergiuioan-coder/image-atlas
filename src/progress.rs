use std::{collections::VecDeque, path::Path, time::Duration};

#[derive(Clone, Copy)]
pub struct Work {
    class: usize,
    units: f64,
}

impl Work {
    pub fn file(path: &Path) -> Self {
        let size = path.metadata().map(|value| value.len()).unwrap_or(0) as f64;
        let dimensions = image::ImageReader::open(path)
            .ok()
            .and_then(|reader| reader.with_guessed_format().ok())
            .and_then(|reader| reader.into_dimensions().ok());
        match dimensions {
            Some((width, height)) => Self {
                class: 0,
                units: (width as f64 * height as f64 / 1e6 + size / 1e7).max(0.01),
            },
            None => Self {
                class: 1,
                units: (size / 1e6).max(0.01),
            },
        }
    }
}

pub struct Exports {
    work: Vec<Work>,
    done: usize,
    remaining: [f64; 2],
    samples: VecDeque<(Work, f64)>,
}

impl Exports {
    pub fn new(work: Vec<Work>) -> Self {
        let mut remaining = [0.0; 2];
        for item in &work {
            remaining[item.class] += item.units;
        }
        Self {
            work,
            remaining,
            done: 0,
            samples: VecDeque::new(),
        }
    }
    pub fn finish(&mut self, elapsed: Duration, success: bool) -> (f64, Option<f64>) {
        if let Some(work) = self.work.get(self.done).copied()
            && success
        {
            self.samples
                .push_back((work, elapsed.as_secs_f64().max(0.000001)));
            if self.samples.len() > 24 {
                self.samples.pop_front();
            }
        }
        if let Some(work) = self.work.get(self.done) {
            self.remaining[work.class] = (self.remaining[work.class] - work.units).max(0.0);
        }
        self.done += 1;
        let seconds: f64 = self.samples.iter().map(|(_, seconds)| seconds).sum();
        let rate = self.samples.len() as f64 / seconds.max(0.000001);
        if self.done >= self.work.len() {
            return (rate, Some(0.0));
        }
        if self.samples.len() < 3 {
            return (rate, None);
        }
        let mut weighted_seconds = [0.0; 2];
        let mut weighted_units = [0.0; 2];
        for (index, (sample, seconds)) in self.samples.iter().enumerate() {
            let weight = (index + 1) as f64;
            weighted_seconds[sample.class] += seconds * weight;
            weighted_units[sample.class] += sample.units * weight;
        }
        let mut remaining = 0.0;
        for (class, units) in self.remaining.iter().enumerate() {
            if *units > 0.000001 {
                if weighted_units[class] == 0.0 {
                    return (rate, None);
                }
                remaining += units * weighted_seconds[class] / weighted_units[class];
            }
        }
        (rate, Some(remaining))
    }
}

pub fn label(done: usize, total: usize, rate: f64, eta: Option<f64>, age: Duration) -> String {
    let eta = match eta {
        Some(seconds) if seconds > age.as_secs_f64() => {
            let left = (seconds - age.as_secs_f64()).ceil() as u64;
            crate::tr!("About {}m {:02}s left", left / 60, left % 60)
        }
        Some(_) if done < total => crate::i18n::text("Optimizing image files…").into(),
        Some(_) => crate::i18n::text("All finished").into(),
        None => crate::i18n::text("Estimating time remaining…").into(),
    };
    let prefix = crate::tr!("Optimizing {done} of {total} images…");
    format!("{prefix} · {rate:.2}/s · {eta}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn larger_remaining_images_increase_the_estimate() {
        let work = vec![
            Work {
                class: 0,
                units: 1.0
            };
            3
        ]
        .into_iter()
        .chain([Work {
            class: 0,
            units: 20.0,
        }])
        .collect();
        let mut estimate = Exports::new(work);
        estimate.finish(Duration::from_secs(1), true);
        estimate.finish(Duration::from_secs(1), true);
        let (_, remaining) = estimate.finish(Duration::from_secs(1), true);
        assert_eq!(remaining, Some(20.0));
    }
    #[test]
    fn recent_slow_work_replaces_an_old_fast_average() {
        let mut estimate = Exports::new(vec![
            Work {
                class: 0,
                units: 1.0
            };
            50
        ]);
        for _ in 0..24 {
            estimate.finish(Duration::from_millis(10), true);
        }
        let (_, before) = estimate.finish(Duration::from_millis(10), true);
        let (_, after) = estimate.finish(Duration::from_secs(10), true);
        assert!(after.unwrap() > before.unwrap() * 10.0);
    }
    #[test]
    fn unknown_formats_and_failed_files_do_not_produce_fake_fast_estimates() {
        let mut estimate = Exports::new(vec![
            Work {
                class: 0,
                units: 1.0,
            },
            Work {
                class: 0,
                units: 1.0,
            },
            Work {
                class: 0,
                units: 1.0,
            },
            Work {
                class: 1,
                units: 1.0,
            },
        ]);
        for _ in 0..3 {
            assert!(estimate.finish(Duration::from_millis(10), true).1.is_none());
        }
        assert_eq!(estimate.finish(Duration::from_secs(1), false).1, Some(0.0));
        assert!(!label(1, 5, 2.0, Some(1.0), Duration::from_secs(2)).contains("MB/s"));
    }
}
