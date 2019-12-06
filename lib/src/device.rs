/*
 * Copyright © 2019 Romeo Calota
 *
 * This program is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2 of the licence, or (at your option) any later version.
 *
 * This software is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with this program; if not, see <http://www.gnu.org/licenses/>.
 *
 * Author: Romeo Calota
 */

use ::libc;

/* FIXME: Not thread safe */
static mut LAST_ASSIGNED_ID: u32 = 0;

#[allow(dead_code)]
#[allow(non_camel_case_types)]
enum Access {
    R_OK = 4, /* Test for read permission.  */
    W_OK = 2, /* Test for write permission.  */
    X_OK = 1, /* Test for execute permission.  */
    F_OK = 0  /* Test for existence.  */
}

#[allow(dead_code)]
#[allow(non_camel_case_types)]
enum FileTypes {
    __S_IFMT   = 0o170000, /* These bits determine file type.  */
    __S_IFDIR  = 0o040000, /* Directory.  */
    __S_IFCHR  = 0o020000, /* Character device.  */
    __S_IFBLK  = 0o060000, /* Block device.  */
    __S_IFREG  = 0o100000, /* Regular file.  */
    __S_IFIFO  = 0o010000, /* FIFO.  */
    __S_IFLNK  = 0o120000, /* Symbolic link.  */
    __S_IFSOCK = 0o140000  /* Socket.  */
}

#[derive(Copy, Clone)]
#[repr(C)]
struct timespec {
    pub tv_sec: libc::c_long,
    pub tv_nsec: libc::c_long,
}

#[derive(Copy, Clone)]
#[repr(C)]
struct stat {
    pub st_dev: libc::c_ulong,
    pub st_ino: libc::c_ulong,
    pub st_nlink: libc::c_ulong,
    pub st_mode: libc::c_uint,
    pub st_uid: libc::c_uint,
    pub st_gid: libc::c_uint,
    pub __pad0: libc::c_int,
    pub st_rdev: libc::c_ulong,
    pub st_size: libc::c_long,
    pub st_blksize: libc::c_long,
    pub st_blocks: libc::c_long,
    pub st_atim: timespec,
    pub st_mtim: timespec,
    pub st_ctim: timespec,
    pub __glibc_reserved: [libc::c_long; 3],
}

extern "C" {
    #[no_mangle]
    fn access(__name: *const libc::c_uchar, __type: libc::c_int)
              -> libc::c_int;
    #[no_mangle]
    fn lstat(__file: *const libc::c_uchar, __buf: *mut stat) -> libc::c_int;
}

#[derive(Clone, Debug)]
pub struct Device
{
    pub id: u32,
    pub path: String,
    pub name: String,
    pub product_id: u32,
    pub vendor_id: u32
}

impl Device
{
    fn new_with_id(id: u32, path: String, name: String, product_id: u32, vendor_id: u32) -> Option<Device> {
        unsafe {
            /* Check if file exists */
            let file_exists = access(path.as_ptr(), Access::F_OK as libc::c_int) != (-1 as libc::c_int);

            let mut sb: stat = std::mem::uninitialized();
            if file_exists && lstat(path.as_ptr(), &mut sb) != (-1 as libc::c_int) {
                if sb.st_mode & FileTypes::__S_IFMT as libc::c_uint == FileTypes::__S_IFCHR as libc::c_uint {
                    let device = Device { id, path, name, product_id, vendor_id };
                    return Some(device);
                }
            }
        }
        None
    }

    pub fn new(path: &str, name: &str, product_id: u32, vendor_id: u32) -> Option<Device> {
        let new_id: u32;
        unsafe {
            LAST_ASSIGNED_ID += 1;
            new_id = LAST_ASSIGNED_ID;
        }
        Device::new_with_id(new_id, String::from(path), String::from(name), product_id, vendor_id)
    }
}
