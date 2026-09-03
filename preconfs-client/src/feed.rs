//! Feeds and regions. A stream serves exactly one feed in one region; the
//! region names are the ones the server reports and the endpoint list uses.

use {
    std::{fmt, str::FromStr},
    triton_preconfs_proto::preconfs::{BamRegion, HarmonicRegion, subscribe_request},
};

/// A preconf source. Each feed has its own set of regions and its own
/// stream service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Feed {
    /// Harmonic: the builder executes the transaction and reports the
    /// outcome, so streams carry execution results and slot framing.
    Harmonic,
    /// BAM: the leader commits the transaction; no execution outcome and
    /// no slot framing, the slot is on each transaction.
    Bam,
}

impl Feed {
    /// Lowercase name, `harmonic` or `bam`.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Harmonic => "harmonic",
            Self::Bam => "bam",
        }
    }

    /// Whether updates carry an execution result, and so whether
    /// `execution_results` filters are accepted.
    pub const fn has_execution_results(self) -> bool {
        matches!(self, Self::Harmonic)
    }

    /// Region names this feed serves, as accepted by [`Region::parse`].
    pub fn regions(self) -> Vec<&'static str> {
        match self {
            Self::Harmonic => HARMONIC.iter().map(|(name, _)| *name).collect(),
            Self::Bam => BAM.iter().map(|(name, _)| *name).collect(),
        }
    }
}

impl FromStr for Feed {
    type Err = RegionError;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        match name.to_ascii_lowercase().as_str() {
            "harmonic" => Ok(Self::Harmonic),
            "bam" => Ok(Self::Bam),
            _ => Err(RegionError::UnknownFeed(name.to_string())),
        }
    }
}

impl fmt::Display for Feed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

const HARMONIC: &[(&str, HarmonicRegion)] = &[
    ("ams", HarmonicRegion::Ams),
    ("ewr", HarmonicRegion::Ewr),
    ("fra", HarmonicRegion::Fra),
    ("lon", HarmonicRegion::Lon),
    ("tyo", HarmonicRegion::Tyo),
    ("sgp", HarmonicRegion::Sgp),
    ("slc", HarmonicRegion::Slc),
];

const BAM: &[(&str, BamRegion)] = &[
    ("ams", BamRegion::Ams),
    ("dfw", BamRegion::Dfw),
    ("dub", BamRegion::Dub),
    ("ewr", BamRegion::Ewr),
    ("fra", BamRegion::Fra),
    ("hkg", BamRegion::Hkg),
    ("iad", BamRegion::Iad),
    ("lax", BamRegion::Lax),
    ("lon", BamRegion::Lon),
    ("pit", BamRegion::Pit),
    ("sea", BamRegion::Sea),
    ("sin", BamRegion::Sin),
    ("slc", BamRegion::Slc),
    ("sqq", BamRegion::Sqq),
    ("tyo", BamRegion::Tyo),
];

/// A feed-typed region, the required part of every subscribe request.
///
/// Build one with [`Region::parse`] or parse `feed:region` such as
/// `harmonic:ams`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Region {
    /// A Harmonic region.
    Harmonic(HarmonicRegion),
    /// A BAM region.
    Bam(BamRegion),
}

/// A feed or region name the client does not know, or a region used with
/// the wrong feed.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RegionError {
    /// Not `harmonic` or `bam`.
    #[error("unknown feed {0}; expected harmonic or bam")]
    UnknownFeed(String),
    /// Not one of the feed's regions.
    #[error("unknown {feed} region {name}; expected one of {expected}")]
    UnknownRegion {
        /// The feed the name was looked up in.
        feed: Feed,
        /// The name given.
        name: String,
        /// The names the feed serves, comma separated.
        expected: String,
    },
    /// Not of the form `feed:region`.
    #[error("expected feed:region such as harmonic:ams, got {0:?}")]
    Format(String),
    /// A region of one feed passed to the other feed's subscribe.
    #[error("{region} is not a {expected} region")]
    WrongFeed {
        /// The feed being subscribed.
        expected: Feed,
        /// The region given.
        region: Region,
    },
}

impl Region {
    /// Resolves a region name such as `ams` for the feed, case insensitive.
    pub fn parse(feed: Feed, name: &str) -> Result<Self, RegionError> {
        let lower = name.to_ascii_lowercase();
        let found = match feed {
            Feed::Harmonic => HARMONIC
                .iter()
                .find(|(candidate, _)| *candidate == lower)
                .map(|(_, region)| Self::Harmonic(*region)),
            Feed::Bam => BAM
                .iter()
                .find(|(candidate, _)| *candidate == lower)
                .map(|(_, region)| Self::Bam(*region)),
        };
        found.ok_or_else(|| RegionError::UnknownRegion {
            feed,
            name: name.to_string(),
            expected: feed.regions().join(", "),
        })
    }

    /// The feed this region belongs to.
    pub const fn feed(self) -> Feed {
        match self {
            Self::Harmonic(_) => Feed::Harmonic,
            Self::Bam(_) => Feed::Bam,
        }
    }

    /// Lowercase region name, `ams` and so on.
    pub fn name(self) -> &'static str {
        match self {
            Self::Harmonic(region) => HARMONIC
                .iter()
                .find(|(_, candidate)| *candidate == region)
                .map_or("unspecified", |(name, _)| name),
            Self::Bam(region) => BAM
                .iter()
                .find(|(_, candidate)| *candidate == region)
                .map_or("unspecified", |(name, _)| name),
        }
    }

    pub(crate) const fn into_proto(self) -> subscribe_request::Region {
        match self {
            Self::Harmonic(region) => subscribe_request::Region::HarmonicRegion(region as i32),
            Self::Bam(region) => subscribe_request::Region::BamRegion(region as i32),
        }
    }
}

/// Parses `feed:region`, such as `harmonic:ams` or `bam:fra`.
impl FromStr for Region {
    type Err = RegionError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (feed, region) = value
            .split_once(':')
            .ok_or_else(|| RegionError::Format(value.to_string()))?;
        Self::parse(feed.parse()?, region)
    }
}

impl fmt::Display for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.feed(), self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_region_round_trips_through_its_name() {
        for feed in [Feed::Harmonic, Feed::Bam] {
            for name in feed.regions() {
                let region = Region::parse(feed, name).unwrap();
                assert_eq!(region.feed(), feed);
                assert_eq!(region.name(), name);
                assert_eq!(Region::parse(feed, &name.to_uppercase()).unwrap(), region);
                assert_eq!(region.to_string().parse::<Region>().unwrap(), region);
            }
        }
        assert_eq!(Feed::Harmonic.regions().len(), 7);
        assert_eq!(Feed::Bam.regions().len(), 15);
    }

    #[test]
    fn unknown_names_are_rejected_with_the_expected_list() {
        let error = Region::parse(Feed::Harmonic, "dfw").unwrap_err();
        assert!(matches!(
            error,
            RegionError::UnknownRegion {
                feed: Feed::Harmonic,
                ..
            }
        ));
        assert!(error.to_string().contains("ams, ewr"));
        assert_eq!(
            "shreds".parse::<Feed>().unwrap_err(),
            RegionError::UnknownFeed("shreds".into())
        );
        assert_eq!(
            "ams".parse::<Region>().unwrap_err(),
            RegionError::Format("ams".into())
        );
    }
}
