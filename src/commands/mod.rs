mod leaderboard;
mod rule;
mod spirit;

use crate::app::{Data, Error};

pub fn all() -> Vec<poise::Command<Data, Error>> {
    vec![rule::rule(), spirit::spirit(), leaderboard::leaderboard()]
}
