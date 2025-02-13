//! Support for creating / parsing ancillary data.

// Copied from PR to the standard library.
// PR: https://github.com/rust-lang/rust/pull/69864
// File downloaded from: https://raw.githubusercontent.com/rust-lang/rust/20c88ddd5fe668b29e8fc2c3838710093e8eb94b/library/std/src/sys/unix/ext/net/ancillary.rs

use core::convert::TryFrom;
use core::marker::PhantomData;
use core::mem::{size_of, zeroed};
use core::ptr::read_unaligned;
use core::slice::from_raw_parts;
use std::os::unix::io::RawFd;

#[cfg(any(target_os = "android", target_os = "linux",))]
use libc::{gid_t, pid_t, uid_t};

fn add_to_ancillary_data<T>(
	buffer: &mut [u8],
	length: &mut usize,
	source: &[T],
	cmsg_level: libc::c_int,
	cmsg_type: libc::c_int,
) -> bool {
	let source_len = if let Some(source_len) = source.len().checked_mul(size_of::<T>()) {
		if let Ok(source_len) = u32::try_from(source_len) {
			source_len
		} else {
			return false;
		}
	} else {
		return false;
	};

	unsafe {
		let additional_space = libc::CMSG_SPACE(source_len) as usize;

		let new_length = if let Some(new_length) = additional_space.checked_add(*length) {
			new_length
		} else {
			return false;
		};

		if new_length > buffer.len() {
			return false;
		}

		for byte in &mut buffer[*length..new_length] {
			*byte = 0;
		}

		*length = new_length;

		let mut msg: libc::msghdr = zeroed();
		msg.msg_control = buffer.as_mut_ptr().cast();
		msg.msg_controllen = *length as _;

		let mut cmsg = libc::CMSG_FIRSTHDR(&msg);
		let mut previous_cmsg = cmsg;
		while !cmsg.is_null() && (*cmsg).cmsg_len > 0 {
			previous_cmsg = cmsg;
			cmsg = libc::CMSG_NXTHDR(&msg, cmsg);
		}

		if previous_cmsg.is_null() {
			return false;
		}

		(*previous_cmsg).cmsg_level = cmsg_level;
		(*previous_cmsg).cmsg_type = cmsg_type;
		(*previous_cmsg).cmsg_len = libc::CMSG_LEN(source_len) as _;

		let data = libc::CMSG_DATA(previous_cmsg).cast();

		libc::memcpy(data, source.as_ptr().cast(), source_len as usize);
	}
	true
}

struct AncillaryDataIter<'a, T> {
	data: &'a [u8],
	phantom: PhantomData<T>,
}

impl<'a, T> AncillaryDataIter<'a, T> {
	/// Create `AncillaryDataIter` struct to iterate through the data unit in the control message.
	///
	/// # Safety
	///
	/// `data` must contain a valid control message.
	unsafe fn new(data: &'a [u8]) -> AncillaryDataIter<'a, T> {
		AncillaryDataIter {
			data,
			phantom: PhantomData,
		}
	}
}

impl<'a, T> Iterator for AncillaryDataIter<'a, T> {
	type Item = T;

	fn next(&mut self) -> Option<T> {
		if size_of::<T>() <= self.data.len() {
			unsafe {
				let unit = read_unaligned(self.data.as_ptr().cast());
				self.data = &self.data[size_of::<T>()..];
				Some(unit)
			}
		} else {
			None
		}
	}
}

/// Unix credential.
#[cfg(any(target_os = "android", target_os = "linux",))]
#[derive(Clone)]
pub struct SocketCred(libc::ucred);

#[cfg(any(target_os = "android", target_os = "linux",))]
impl SocketCred {
	/// Create a Unix credential struct.
	///
	/// PID, UID and GID is set to 0.
	#[allow(clippy::new_without_default)]
	pub fn new() -> SocketCred {
		SocketCred(libc::ucred { pid: 0, uid: 0, gid: 0 })
	}

	/// Set the PID.
	pub fn set_pid(&mut self, pid: pid_t) {
		self.0.pid = pid;
	}

	/// Get the current PID.
	pub fn get_pid(&self) -> pid_t {
		self.0.pid
	}

	/// Set the UID.
	pub fn set_uid(&mut self, uid: uid_t) {
		self.0.uid = uid;
	}

	/// Get the current UID.
	pub fn get_uid(&self) -> uid_t {
		self.0.uid
	}

	/// Set the GID.
	pub fn set_gid(&mut self, gid: gid_t) {
		self.0.gid = gid;
	}

	/// Get the current GID.
	pub fn get_gid(&self) -> gid_t {
		self.0.gid
	}
}

/// This control message contains file descriptors.
///
/// The level is equal to `SOL_SOCKET` and the type is equal to `SCM_RIGHTS`.
pub struct ScmRights<'a>(AncillaryDataIter<'a, RawFd>);

impl<'a> Iterator for ScmRights<'a> {
	type Item = RawFd;

	fn next(&mut self) -> Option<RawFd> {
		self.0.next()
	}
}

/// This control message contains unix credentials.
///
/// The level is equal to `SOL_SOCKET` and the type is equal to `SCM_CREDENTIALS` or `SCM_CREDS`.
#[cfg(any(target_os = "android", target_os = "linux",))]
pub struct ScmCredentials<'a>(AncillaryDataIter<'a, libc::ucred>);

#[cfg(any(target_os = "android", target_os = "linux",))]
impl<'a> Iterator for ScmCredentials<'a> {
	type Item = SocketCred;

	fn next(&mut self) -> Option<SocketCred> {
		Some(SocketCred(self.0.next()?))
	}
}

/// The error type which is returned from parsing the type a control message.
#[non_exhaustive]
#[derive(Debug)]
pub enum AncillaryError {
	/// The ancillary data type is not recognized.
	Unknown {
		/// The cmsg_level field of the ancillary data.
		cmsg_level: i32,

		/// The cmsg_type field of the ancillary data.
		cmsg_type: i32,
	},
}

/// This enum represent one control message of variable type.
pub enum AncillaryData<'a> {
	/// Ancillary data holding file descriptors.
	ScmRights(ScmRights<'a>),

	/// Ancillary data holding unix credentials.
	#[cfg(any(target_os = "android", target_os = "linux",))]
	ScmCredentials(ScmCredentials<'a>),
}

impl<'a> AncillaryData<'a> {
	/// Create a `AncillaryData::ScmRights` variant.
	///
	/// # Safety
	///
	/// `data` must contain a valid control message and the control message must be type of
	/// `SOL_SOCKET` and level of `SCM_RIGHTS`.
	#[allow(clippy::wrong_self_convention)]
	unsafe fn as_rights(data: &'a [u8]) -> Self {
		let ancillary_data_iter = AncillaryDataIter::new(data);
		let scm_rights = ScmRights(ancillary_data_iter);
		AncillaryData::ScmRights(scm_rights)
	}

	/// Create a `AncillaryData::ScmCredentials` variant.
	///
	/// # Safety
	///
	/// `data` must contain a valid control message and the control message must be type of
	/// `SOL_SOCKET` and level of `SCM_CREDENTIALS` or `SCM_CREDENTIALS`.
	#[cfg(any(target_os = "android", target_os = "linux",))]
	#[allow(clippy::wrong_self_convention)]
	unsafe fn as_credentials(data: &'a [u8]) -> Self {
		let ancillary_data_iter = AncillaryDataIter::new(data);
		let scm_credentials = ScmCredentials(ancillary_data_iter);
		AncillaryData::ScmCredentials(scm_credentials)
	}

	fn try_from_cmsghdr(cmsg: &'a libc::cmsghdr) -> Result<Self, AncillaryError> {
		unsafe {
			let cmsg_len_zero = libc::CMSG_LEN(0);
			let data_len = cmsg.cmsg_len as usize - cmsg_len_zero as usize;
			let data = libc::CMSG_DATA(cmsg).cast();
			let data = from_raw_parts(data, data_len);

			match cmsg.cmsg_level {
				libc::SOL_SOCKET => match cmsg.cmsg_type {
					libc::SCM_RIGHTS => Ok(AncillaryData::as_rights(data)),
					#[cfg(any(target_os = "android", target_os = "linux",))]
					libc::SCM_CREDENTIALS => Ok(AncillaryData::as_credentials(data)),
					cmsg_type => Err(AncillaryError::Unknown {
						cmsg_level: libc::SOL_SOCKET,
						cmsg_type,
					}),
				},
				cmsg_level => Err(AncillaryError::Unknown {
					cmsg_level,
					cmsg_type: cmsg.cmsg_type,
				}),
			}
		}
	}
}

/// This struct is used to iterate through the control messages.
pub struct Messages<'a> {
	buffer: &'a [u8],
	current: Option<&'a libc::cmsghdr>,
}

impl<'a> Iterator for Messages<'a> {
	type Item = Result<AncillaryData<'a>, AncillaryError>;

	fn next(&mut self) -> Option<Self::Item> {
		unsafe {
			let mut msg: libc::msghdr = zeroed();
			msg.msg_control = self.buffer.as_ptr() as *mut _;
			msg.msg_controllen = self.buffer.len() as _;

			let cmsg = if let Some(current) = self.current {
				libc::CMSG_NXTHDR(&msg, current)
			} else {
				libc::CMSG_FIRSTHDR(&msg)
			};

			let cmsg = cmsg.as_ref()?;
			self.current = Some(cmsg);
			let ancillary_result = AncillaryData::try_from_cmsghdr(cmsg);
			Some(ancillary_result)
		}
	}
}

/// A Unix socket Ancillary data struct.
#[derive(Debug)]
pub struct SocketAncillary<'a> {
	pub(crate) buffer: &'a mut [u8],
	pub(crate) length: usize,
	pub(crate) truncated: bool,
}

impl<'a> SocketAncillary<'a> {
	/// Create an ancillary data with the given buffer.
	///
	/// # Example
	///
	/// ```no_run
	/// use tokio_seqpacket::ancillary::SocketAncillary;
	/// let mut ancillary_buffer = [0; 128];
	/// let mut ancillary = SocketAncillary::new(&mut ancillary_buffer[..]);
	/// ```
	pub fn new(buffer: &'a mut [u8]) -> Self {
		SocketAncillary {
			buffer,
			length: 0,
			truncated: false,
		}
	}

	/// Returns the capacity of the buffer.
	pub fn capacity(&self) -> usize {
		self.buffer.len()
	}

	/// Returns the number of used bytes.
	pub fn len(&self) -> usize {
		self.length
	}

	/// Is `true` if the number of used bytes is zero.
	pub fn is_empty(&self) -> bool {
		self.length == 0
	}

	/// Returns the iterator of the control messages.
	pub fn messages(&self) -> Messages<'_> {
		Messages {
			buffer: &self.buffer[..self.length],
			current: None,
		}
	}

	/// Is `true` if during a recv operation the ancillary was truncated.
	pub fn truncated(&self) -> bool {
		self.truncated
	}

	/// Add file descriptors to the ancillary data.
	///
	/// The function returns `true` if there was enough space in the buffer.
	/// If there was not enough space then no file descriptors was appended.
	/// Technically, that means this operation adds a control message with the level `SOL_SOCKET`
	/// and type `SCM_RIGHTS`.
	pub fn add_fds(&mut self, fds: &[RawFd]) -> bool {
		self.truncated = false;
		add_to_ancillary_data(self.buffer, &mut self.length, fds, libc::SOL_SOCKET, libc::SCM_RIGHTS)
	}

	/// Add credentials to the ancillary data.
	///
	/// The function returns `true` if there was enough space in the buffer.
	/// If there was not enough space then no credentials was appended.
	/// Technically, that means this operation adds a control message with the level `SOL_SOCKET`
	/// and type `SCM_CREDENTIALS` or `SCM_CREDS`.
	///
	#[cfg(any(target_os = "android", target_os = "linux",))]
	pub fn add_creds(&mut self, creds: &[SocketCred]) -> bool {
		self.truncated = false;
		add_to_ancillary_data(
			self.buffer,
			&mut self.length,
			creds,
			libc::SOL_SOCKET,
			libc::SCM_CREDENTIALS,
		)
	}

	/// Clears the ancillary data, removing all values.
	pub fn clear(&mut self) {
		self.length = 0;
		self.truncated = false;
	}
}
