mod rule;
mod spirit;
mod stats;

use crate::app::{Data, Error};

pub fn all() -> Vec<poise::Command<Data, Error>> {
    vec![rule::rule(), spirit::spirit(), stats::stats()]
}
