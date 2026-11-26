#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Generation(u64);

impl Generation {
    fn next(self) -> Self {
        Self(
            self.0
                .checked_add(1)
                .expect("lifecycle generation counter exhausted"),
        )
    }

    pub(crate) fn value(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Lifecycle<P> {
    generation: Generation,
    phase: P,
}

impl<P: Copy> Lifecycle<P> {
    pub(crate) fn new(phase: P) -> Self {
        Self {
            generation: Generation::default(),
            phase,
        }
    }

    pub(crate) fn begin(&mut self, phase: P) -> Generation {
        self.generation = self.generation.next();
        self.phase = phase;
        self.generation
    }

    pub(crate) fn apply(&mut self, generation: Generation, phase: P) -> bool {
        if self.generation != generation {
            return false;
        }
        self.phase = phase;
        true
    }

    pub(crate) fn generation(&self) -> Generation {
        self.generation
    }

    pub(crate) fn phase(&self) -> P {
        self.phase
    }
}
