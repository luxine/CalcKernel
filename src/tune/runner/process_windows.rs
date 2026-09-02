use std::process::Command;

pub(super) fn configure(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x0000_0200); // CREATE_NEW_PROCESS_GROUP
}

pub(super) struct Containment(windows_sys::Win32::Foundation::HANDLE);

pub(super) fn establish(child: &std::process::Child) -> Result<Containment, std::io::Error> {
    use std::mem::size_of;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        },
    };
    // SAFETY: null security/name creates one private unnamed job owned below.
    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job.is_null() || job == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: the buffer has the exact documented structure and size.
    let configured = unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            (&raw const limits).cast(),
            u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
                .expect("job information size fits u32"),
        )
    };
    let process = child.as_raw_handle().cast();
    // SAFETY: both handles remain valid for this call.
    let assigned = configured != 0 && unsafe { AssignProcessToJobObject(job, process) } != 0;
    if !assigned {
        // SAFETY: job is uniquely owned here.
        unsafe { CloseHandle(job) };
        return Err(std::io::Error::last_os_error());
    }
    Ok(Containment(job))
}

impl Containment {
    pub(super) fn terminate(&self) {
        use windows_sys::Win32::System::JobObjects::TerminateJobObject;
        // SAFETY: the handle is a live private Job Object.
        unsafe { TerminateJobObject(self.0, 1) };
    }
}

impl Drop for Containment {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        // SAFETY: this type uniquely owns the job handle.
        unsafe { CloseHandle(self.0) };
    }
}
