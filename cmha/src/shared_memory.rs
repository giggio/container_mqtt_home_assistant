use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::Duration;

use raw_sync::locks::{LockImpl, LockInit, Mutex};
use shared_memory::{Shmem, ShmemConf, ShmemError};

pub type Result<T> = std::result::Result<T, Error>;

pub struct SharedMemory<T>
where
    T: Default + Copy,
{
    shmem: Shmem, // need to hold a reference to shmem or the mutex will fail as the shmem is deleted on drop
    phantom: PhantomData<T>,
}

impl<T> SharedMemory<T>
where
    T: Default + Copy,
{
    pub fn new(shmem_flink: &str) -> Result<SharedMemory<T>> {
        trace!(category = "shared_memory"; "Creating shmem flink {shmem_flink}...");
        Ok(match ShmemConf::new().size(4096).flink(shmem_flink).create() {
            Ok(shmem) => {
                trace!(category = "shared_memory"; "Shmem link created, opening shmem flink {shmem_flink}");
                let shared_memory = SharedMemory::<T> {
                    shmem,
                    phantom: PhantomData,
                };
                trace!(category = "shared_memory"; "Created shmem flink {shmem_flink}, now setting default value.");
                shared_memory.set(T::default())?;
                shared_memory
            }
            Err(ShmemError::LinkExists) => {
                trace!(category = "shared_memory"; "Shmem link already exists, opening shmem flink {shmem_flink}");
                let shmem = ShmemConf::new()
                    .flink(shmem_flink)
                    .open()
                    .map_err(|e| Error::OpenShmemFlink(shmem_flink.to_string(), e))?;
                trace!(category = "shared_memory"; "Opened shmem flink {shmem_flink}");
                SharedMemory::<T> {
                    shmem,
                    phantom: PhantomData,
                }
            }
            Err(e) => {
                error!(category = "shared_memory"; "Unable to create or open shmem flink {shmem_flink} : {e}");
                return Err(Error::CreateOrOpenShmemFlink(shmem_flink.to_string(), e));
            }
        })
    }

    pub fn get(&self) -> Result<T> {
        let mutex = self.create_mutex()?;
        if let Ok(guard) = mutex.lock() {
            let val = unsafe { &*(*guard).cast_const().cast::<T>() };
            Ok(*val)
        } else {
            error!(category = "shared_memory"; "Failed to acquire lock when getting.");
            Err(Error::MutexAquireLock(
                "Failed to acquire lock when getting.".to_string(),
            ))
        }
    }

    pub fn set(&self, value: T) -> Result<()> {
        let mutex = self.create_mutex()?;
        let Ok(guard) = mutex.lock() else {
            error!(category = "shared_memory"; "Failed to acquire lock when setting.");
            return Err(Error::MutexAquireLock(
                "Failed to acquire lock when setting.".to_string(),
            ));
        };
        let val = unsafe { &mut *(*guard).cast::<u8>().cast::<T>() };
        *val = value;
        Ok(())
    }

    fn create_mutex(&self) -> Result<Box<dyn LockImpl>> {
        let mut raw_ptr = self.shmem.as_ptr();
        let is_init: &mut AtomicBool;

        unsafe {
            is_init = &mut *raw_ptr.cast::<u8>().cast::<AtomicBool>();
            raw_ptr = raw_ptr.add(size_of::<AtomicBool>() * 8);
        };
        let mutex = if self.shmem.is_owner() {
            trace!(category = "shared_memory"; "This is the owner of the shared memory, initializing mutex");
            // Initialize the mutex
            let (lock, _bytes_used) = unsafe {
                Mutex::new(
                    raw_ptr,                                    // Base address of Mutex
                    raw_ptr.add(Mutex::size_of(Some(raw_ptr))), // Address of data protected by mutex
                )
                .map_err(|e| Error::MutexInitialization(e.to_string()))?
            };
            trace!(category = "shared_memory"; "Mutex initialized");
            is_init.store(true, Ordering::Release);
            lock
        } else {
            trace!(category = "shared_memory"; "This is not the owner of the shared memory, waiting for mutex to be initialized");
            if !is_init.load(Ordering::Acquire) {
                trace!(category = "shared_memory"; "Waiting for mutex to be initialized...");
                while !is_init.load(Ordering::Acquire) {
                    sleep(Duration::from_millis(100));
                    std::thread::yield_now();
                }
                trace!(category = "shared_memory"; "Done waiting for mutex to be initialized!");
            }
            let (lock, _bytes_used) = unsafe {
                trace!(category = "shared_memory"; "Creating mutex from existing one");
                Mutex::from_existing(
                    raw_ptr,                                    // Base address of Mutex
                    raw_ptr.add(Mutex::size_of(Some(raw_ptr))), // Address of data  protected by mutex
                )
                .map_err(|e| Error::MutexCreateFromExisting(e.to_string()))?
            };
            lock
        };
        Ok(mutex)
    }
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Failed to acquire lock for mutex: {0}")]
    MutexAquireLock(String),
    #[error("Failed to create mutex from an existing one: {0}")]
    MutexCreateFromExisting(String),
    #[error("Failed to initialize mutex: {0}")]
    MutexInitialization(String),
    #[error("Unable to open shmem flink {0} : {1}")]
    OpenShmemFlink(String, ShmemError),
    #[error("Unable to create or open shmem flink {0} : {1}")]
    CreateOrOpenShmemFlink(String, ShmemError),
}
