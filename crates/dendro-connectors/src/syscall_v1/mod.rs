//! Reserved adapter boundary; no parser or producer admission is claimed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    ParserNotImplemented,
}
pub const AVAILABILITY: Availability = Availability::ParserNotImplemented;
