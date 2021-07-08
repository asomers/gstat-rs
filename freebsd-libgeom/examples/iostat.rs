//! Similar to "iostat -x".  See iostat(8).

use freebsd_libgeom::*;
use nix::time::{ClockId, clock_gettime};
use std::{
    error::Error,
};

fn main() -> Result<(), Box<dyn Error>> {
    let mut tree = Tree::new()?;

    let mut initial = Snapshot::new()?;
    println!("{:8}{:>8}{:>8}{:>9}{:>9}{:>6}{:>6}{:>6}{:>6}{:>5}{:>4}",
             "device",
             "r/s",
             "w/s",
             "kr/s",
             "kw/s",
             "ms/r",
             "ms/w",
             "ms/o",
             "ms/t",
             "qlen",
             "%b"
             );
    let boottime = clock_gettime(ClockId::CLOCK_UPTIME)?;
    let boottime_secs = boottime.tv_sec() as f64 + boottime.tv_nsec() as f64 * 1e-9;
    for devstat in &mut initial {
        if let Some(gident) = tree.lookup(devstat.id()) {
            if let Some(1) = gident.rank() {
                let stats = Statistics::compute(devstat, None, boottime_secs);
                println!("{:8} {:>7.0} {:>7.0} {:>8.1} {:>8.1} {:>5.0} {:>5.0} {:>5.0} {:>5.0} {:>4} {:>3.0}",
                    gident.name().to_string_lossy(),
                    stats.transfers_per_second_read(),
                    stats.transfers_per_second_write(),
                    stats.mb_per_second_read() * 1024.0,
                    stats.mb_per_second_write() * 1024.0,
                    stats.ms_per_transaction_read(),
                    stats.ms_per_transaction_write(),
                    stats.ms_per_transaction_other() + stats.ms_per_transaction_free(),
                    stats.ms_per_transaction(),
                    stats.queue_length(),
                    stats.busy_pct()
               )
            }
        }
    }

    Ok(())
}
