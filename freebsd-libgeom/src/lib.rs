/// Safe bindings to FreeBSD's libgeom
///
/// https://www.freebsd.org/cgi/man.cgi?query=libgeom

use freebsd_libgeom_sys::*;
use lazy_static::lazy_static;
use std::{
    ffi::CStr,
    io::{Error, Result},
    marker::PhantomData,
    mem::{self, MaybeUninit},
    ops::Sub,
    os::raw::c_void,
    pin::Pin,
    ptr::NonNull
};

/// Used by [`Statistics::compute`]
macro_rules! delta {
    ($current: ident, $previous: ident, $field:ident, $index:expr) => {
        {
            let idx = $index as usize;
            let old = if let Some(prev) = $previous {
                unsafe {prev.devstat.as_ref() }.$field[idx]
            } else {
                0
            };
            let new = unsafe {$current.devstat.as_ref() }.$field[idx];
            new - old
        }
    }
}

macro_rules! delta_t {
    ($cur: expr, $prev: expr, $bintime:expr) => {
        {
            // BINTIME_SCALE is 1 / 2**64
            const BINTIME_SCALE: f64 = 5.42101086242752217003726400434970855712890625e-20;
            let old: bintime = if let Some(prev) = $prev {
                $bintime(unsafe {prev.devstat.as_ref() })
            } else {
                bintime{sec: 0, frac: 0}
            };
            let new: bintime = $bintime(unsafe {$cur.devstat.as_ref() });
            (new.sec - old.sec) as f64
                + (new.frac - old.frac) as f64 * BINTIME_SCALE
        }
    }
}

macro_rules! fields {
    ($self: ident, $meth: ident, $field: ident) => {
        pub fn $meth(&$self) -> u64 {
            $self.$field
        }
    }
}

macro_rules! fields_per_sec {
    ($self: ident, $meth: ident, $field: ident) => {
        pub fn $meth(&$self) -> f64 {
            if $self.etime > 0.0 {
                $self.$field as f64 / $self.etime
            } else {
                0.0
            }
        }
    }
}

macro_rules! kb_per_xfer {
    ($self: ident, $meth: ident, $xfers: ident, $bytes: ident) => {
        pub fn $meth(&$self) -> f64 {
            if $self.$xfers > 0 {
                $self.$bytes as f64 / (1<<10) as f64 / $self.$xfers as f64
            } else {
                0.0
            }
        }
    }
}

macro_rules! mb_per_sec {
    ($self: ident, $meth: ident, $field: ident) => {
        pub fn $meth(&$self) -> f64 {
            if $self.etime > 0.0 {
                $self.$field as f64 / (1<<20) as f64 / $self.etime
            } else {
                0.0
            }
        }
    }
}

macro_rules! ms_per_xfer {
    ($self: ident, $meth: ident, $xfers: ident, $duration: ident) => {
        pub fn $meth(&$self) -> f64 {
            if $self.$xfers > 0 {
                $self.$duration * 1000.0 / $self.$xfers as f64
            } else {
                0.0
            }
        }
    }
}

lazy_static! {
    static ref GEOM_STATS: Result<()> = {
        let r = unsafe { geom_stats_open() };
        if r != 0 {
            Err(Error::last_os_error())
        } else {
            Ok(())
        }
    };
}

/// Describes the stats of a single geom element as part of a [`Snapshot`].
#[derive(Debug, Copy, Clone)]
#[repr(transparent)]
pub struct Devstat<'a>{
    devstat: NonNull<devstat>,
    phantom: PhantomData<&'a devstat>
}

impl<'a> Devstat<'a> {
    pub fn id(&'a self) -> Id<'a> {
        Id {
            id: unsafe { self.devstat.as_ref() }.id,
            phantom: PhantomData
        }
    }
}

/// Identifies an element in the Geom [`Tree`]
#[derive(Debug, Copy, Clone)]
pub struct Gident<'a>{
    ident: NonNull<gident>,
    phantom: PhantomData<&'a Tree>
}

impl<'a> Gident<'a> {
    pub fn is_consumer(&self) -> bool {
        unsafe{self.ident.as_ref()}.lg_what == gident_ISCONSUMER
    }

    pub fn is_provider(&self) -> bool {
        unsafe{self.ident.as_ref()}.lg_what == gident_ISPROVIDER
    }

    pub fn name(&self) -> &'a CStr {
        unsafe{
            let gprovider = self.ident.as_ref().lg_ptr as *const gprovider;
            assert!(!gprovider.is_null());
            CStr::from_ptr((*gprovider).lg_name)
        }
    }

    pub fn rank(&self) -> Option<u32> {
        unsafe{
            let gprovider = self.ident.as_ref().lg_ptr as *const gprovider;
            assert!(!gprovider.is_null());
            let geom = (*gprovider).lg_geom;
            if geom.is_null() {
                None
            } else {
                Some((*geom).lg_rank)
            }
        }
    }
}

/// A device identifier as contained in `struct devstat`.
#[derive(Debug, Copy, Clone)]
pub struct Id<'a> {
    id: *const c_void,
    phantom: PhantomData<&'a Devstat<'a>>
}

/// A geom statistics snapshot.
///
// FreeBSD BUG: geom_stats_snapshot_get should return an opaque pointer instead
// of a void*, for better type safety.
pub struct Snapshot(NonNull<c_void>);

impl Snapshot {
    /// Iterate through all devices described by the snapshot
    pub fn iter<'a>(&'a mut self) -> SnapshotIter<'a> {
        SnapshotIter(self)
    }

    /// Acquires a new snapshot of the raw data from the kernel.
    ///
    /// Is not guaranteed to be completely atomic and consistent.
    pub fn new() -> Result<Self> {
        GEOM_STATS.as_ref().unwrap();
        let raw = unsafe { geom_stats_snapshot_get() };
        NonNull::new(raw)
            .map(Snapshot)
            .ok_or_else(Error::last_os_error)
    }

    /// Reset the state of the internal iterator back to the beginning
    fn reset(&mut self) {
        unsafe {geom_stats_snapshot_reset(self.0.as_mut())}
    }

    /// Accessor for the embedded timestamp generated by [`Snapshot::new`].
    // FreeBSD BUG: geom_stats_snapshot_timestamp should take a const pointer,
    // not a mut one.
    pub fn timestamp(&mut self) -> Timespec {
        let inner = unsafe {
            let mut ts = MaybeUninit::uninit();
            geom_stats_snapshot_timestamp(self.0.as_mut(), ts.as_mut_ptr());
            ts.assume_init()
        };
        Timespec(inner)
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        unsafe { geom_stats_snapshot_free(self.0.as_mut()) };
    }
}

/// Return type of [`Snapshot::iter`].
pub struct SnapshotIter<'a>(&'a mut Snapshot);

impl<'a> Iterator for SnapshotIter<'a> {
    type Item = Devstat<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let raw = unsafe {geom_stats_snapshot_next(self.0.0.as_mut()) };
        NonNull::new(raw)
            .map(|devstat| Devstat{devstat, phantom: PhantomData})
    }
}

impl<'a> Drop for SnapshotIter<'a> {
    fn drop(&mut self) {
        self.0.reset();
    }
}

/// Computes statistics between two [`Snapshot`]s for the same device.
///
/// This is equivalent to libgeom's
/// [`devstat_compute_statistics`](https://www.freebsd.org/cgi/man.cgi?query=devstat&sektion=3)
/// function.
// Note that Rust cannot bind to devstat_compute_statistics because its API
// includes "long double", which has no Rust equivalent.  So we reimplement the
// logic here.
pub struct Statistics<'a>{
    current: Devstat<'a>,
    previous: Option<Devstat<'a>>,
    etime: f64,
    total_bytes: u64,
    total_bytes_free: u64,
    total_bytes_read: u64,
    total_bytes_write: u64,
    total_blocks: u64,
    total_blocks_free: u64,
    total_blocks_read: u64,
    total_blocks_write: u64,
    total_duration: f64,
    total_duration_free: f64,
    total_duration_other: f64,
    total_duration_read: f64,
    total_duration_write: f64,
    total_transfers: u64,
    total_transfers_free: u64,
    total_transfers_other: u64,
    total_transfers_read: u64,
    total_transfers_write: u64,
}

impl<'a> Statistics<'a> {
    /// Compute statistics between two [`Devstat`] objects, which must
    /// correspond to the same device, and should come from two separate
    /// snapshots
    ///
    /// If `prev` is `None`, then statistics since boot will be returned.
    /// `etime` should be the elapsed time in seconds between the two snapshots.
    pub fn compute(
        current: Devstat<'a>,
        previous: Option<Devstat<'a>>,
        etime: f64) -> Self
    {
        let cur = unsafe { current.devstat.as_ref() };

        let total_transfers_read = delta!(current, previous, operations,
                                          devstat_trans_flags_DEVSTAT_READ);
        let total_transfers_write = delta!(current, previous, operations,
                                           devstat_trans_flags_DEVSTAT_WRITE);
        let total_transfers_other = delta!(current, previous, operations,
                                           devstat_trans_flags_DEVSTAT_NO_DATA);
        let total_transfers_free = delta!(current, previous, operations,
                                          devstat_trans_flags_DEVSTAT_FREE);
        let total_transfers = total_transfers_read + total_transfers_write +
            total_transfers_other + total_transfers_free;

        let total_bytes_free = delta!(current, previous, bytes,
                                          devstat_trans_flags_DEVSTAT_FREE);
        let total_bytes_read = delta!(current, previous, bytes,
                                          devstat_trans_flags_DEVSTAT_READ);
        let total_bytes_write = delta!(current, previous, bytes,
                                          devstat_trans_flags_DEVSTAT_WRITE);
        let total_bytes = total_bytes_read + total_bytes_write +
            total_bytes_free;

        let block_denominator = if cur.block_size > 0 {
            cur.block_size as u64
        } else {
            512u64
        };
        let total_blocks = total_bytes / block_denominator;
        let total_blocks_free = total_bytes_free / block_denominator;
        let total_blocks_read = total_bytes_read / block_denominator;
        let total_blocks_write = total_bytes_write / block_denominator;

        let total_duration_free = delta_t!(current, previous,
            |ds: &devstat|
                ds.duration[devstat_trans_flags_DEVSTAT_FREE as usize]
        );
        let total_duration_read = delta_t!(current, previous,
            |ds: &devstat|
                ds.duration[devstat_trans_flags_DEVSTAT_READ as usize]
        );
        let total_duration_write = delta_t!(current, previous,
            |ds: &devstat|
                ds.duration[devstat_trans_flags_DEVSTAT_WRITE as usize]
        );
        let total_duration_other = delta_t!(current, previous,
            |ds: &devstat|
                ds.duration[devstat_trans_flags_DEVSTAT_NO_DATA as usize]
        );
        let total_duration = total_duration_read + total_duration_write +
            total_duration_other + total_duration_free;

        Self{
            current,
            previous,
            etime,
            total_bytes,
            total_bytes_free,
            total_bytes_read,
            total_bytes_write,
            total_blocks,
            total_blocks_free,
            total_blocks_read,
            total_blocks_write,
            total_duration,
            total_duration_free,
            total_duration_other,
            total_duration_read,
            total_duration_write,
            total_transfers,
            total_transfers_free,
            total_transfers_other,
            total_transfers_read,
            total_transfers_write,
        }
    }

    /// The percentage of time the device had one or more transactions
    /// outstanding between the acquisition of the two snapshots.
    pub fn busy_pct(&self) -> f64 {
        let delta = delta_t!(&self.current, &self.previous,
            |ds: &devstat| ds.busy_time);
        (delta / self.etime * 100.0).max(0.0)
    }

    /// Returns the number of incomplete transactions at the time `cur` was
    /// acquired.
    pub fn queue_length(&self) -> u32 {
        let cur = unsafe {self.current.devstat.as_ref() };
        return cur.start_count - cur.end_count
    }

    fields!{self, total_bytes, total_bytes}
    fields!{self, total_bytes_free, total_bytes_free}
    fields!{self, total_bytes_read, total_bytes_read}
    fields!{self, total_bytes_write, total_bytes_write}
    fields!{self, total_blocks, total_blocks}
    fields!{self, total_blocks_free, total_blocks_free}
    fields!{self, total_blocks_read, total_blocks_read}
    fields!{self, total_blocks_write, total_blocks_write}
    fields!{self, total_transfers, total_transfers}
    fields!{self, total_transfers_free, total_transfers_free}
    fields!{self, total_transfers_read, total_transfers_read}
    fields!{self, total_transfers_other, total_transfers_other}
    fields!{self, total_transfers_write, total_transfers_write}
    fields_per_sec!{self, blocks_per_second, total_blocks}
    fields_per_sec!{self, blocks_per_second_free, total_blocks_free}
    fields_per_sec!{self, blocks_per_second_read, total_blocks_read}
    fields_per_sec!{self, blocks_per_second_write, total_blocks_write}
    kb_per_xfer!{self, kb_per_transfer, total_transfers, total_bytes}
    kb_per_xfer!{self, kb_per_transfer_free, total_transfers_free, total_bytes}
    kb_per_xfer!{self, kb_per_transfer_read, total_transfers_read, total_bytes}
    kb_per_xfer!{self, kb_per_transfer_write, total_transfers_write,
        total_bytes}
    ms_per_xfer!{self, ms_per_transaction, total_transfers, total_duration}
    ms_per_xfer!{self, ms_per_transaction_free, total_transfers_free,
                total_duration_free}
    ms_per_xfer!{self, ms_per_transaction_read, total_transfers_read,
                total_duration_read}
    ms_per_xfer!{self, ms_per_transaction_other, total_transfers_other,
                total_duration_other}
    ms_per_xfer!{self, ms_per_transaction_write, total_transfers_write,
                total_duration_write}
    mb_per_sec!{self, mb_per_second, total_bytes}
    mb_per_sec!{self, mb_per_second_free, total_bytes_free}
    mb_per_sec!{self, mb_per_second_read, total_bytes_read}
    mb_per_sec!{self, mb_per_second_write, total_bytes_write}
    fields_per_sec!{self, transfers_per_second, total_transfers}
    fields_per_sec!{self, transfers_per_second_free, total_transfers_free}
    fields_per_sec!{self, transfers_per_second_other, total_transfers_other}
    fields_per_sec!{self, transfers_per_second_read, total_transfers_read}
    fields_per_sec!{self, transfers_per_second_write, total_transfers_write}
}

/// Return type of [`Snapshot::timestamp`].  It's the familiar C `timespec`.
#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
// The wrapper is necessary just to be proper CamelCase
pub struct Timespec(freebsd_libgeom_sys::timespec);

impl From<Timespec> for f64 {
    fn from(ts: Timespec) -> f64 {
        ts.0.tv_sec as f64 + ts.0.tv_nsec as f64 * 1e-9
    }
}

impl Sub for Timespec {
    type Output = Self;

    fn sub(self, rhs: Timespec) -> Self::Output {
        let mut tv_sec = self.0.tv_sec - rhs.0.tv_sec;
        let mut tv_nsec = self.0.tv_nsec - rhs.0.tv_nsec;
        if tv_nsec < 0 {
            tv_sec -= 1;
            tv_nsec += 1_000_000_000;
        }
        Self(freebsd_libgeom_sys::timespec {tv_sec, tv_nsec})
    }
}

/// Describes the entire Geom heirarchy.
#[derive(Debug)]
#[repr(transparent)]
pub struct Tree(Pin<Box<gmesh>>);

impl Tree {
    // FreeBSD BUG: geom_lookupid takes a mutable pointer when it could be const
    pub fn lookup<'a>(&'a mut self, id: Id) -> Option<Gident<'a>> {
        let raw = unsafe {
            geom_lookupid(&mut *self.0, id.id)
        };
        NonNull::new(raw)
            .map(|ident| Gident{ident, phantom: PhantomData})
    }

    /// Construct a new `Tree` representing all available geom providers
    pub fn new() -> Result<Self> {
        let (inner, r) = unsafe {
            let mut inner = Box::pin(mem::zeroed());
            let r = geom_gettree(&mut *inner);
            (inner, r)
        };
        if r != 0 {
            Err(Error::last_os_error())
        } else {
            Ok(Tree(inner))
        }
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        unsafe { geom_deletetree(&mut *self.0) };
    }
}
