use hara_runtime::interpreter_observation::{invoke_json, ABI_VERSION};

#[no_mangle]
pub extern "C" fn interpreter_observation_abi_version() -> i32 {
    ABI_VERSION
}

#[no_mangle]
pub extern "C" fn interpreter_observation_alloc(size: usize) -> *mut u8 {
    allocate_bytes(size)
}

#[no_mangle]
pub extern "C" fn interpreter_observation_dealloc(pointer: *mut u8, size: usize) {
    free_bytes(pointer, size);
}

/// Accepts one UTF-8 JSON request and returns packed `(pointer << 32) | len`.
#[no_mangle]
pub extern "C" fn interpreter_observation_invoke(pointer: *const u8, size: usize) -> u64 {
    let response = if pointer.is_null() {
        invoke_json("{}")
    } else {
        let bytes = unsafe { std::slice::from_raw_parts(pointer, size) };
        match std::str::from_utf8(bytes) {
            Ok(source) => invoke_json(source),
            Err(_) => invoke_json("{}"),
        }
    };
    pack_response(response)
}

fn allocate_bytes(size: usize) -> *mut u8 {
    let bytes = vec![0_u8; size.max(1)].into_boxed_slice();
    Box::into_raw(bytes) as *mut u8
}

fn free_bytes(pointer: *mut u8, size: usize) {
    if pointer.is_null() {
        return;
    }
    let length = size.max(1);
    unsafe {
        let slice = std::ptr::slice_from_raw_parts_mut(pointer, length);
        drop(Box::from_raw(slice));
    }
}

fn pack_response(bytes: Vec<u8>) -> u64 {
    let length = bytes.len();
    let pointer = Box::into_raw(bytes.into_boxed_slice()) as *mut u8;
    ((pointer as u64) << 32) | length as u64
}

#[cfg(test)]
mod tests {
    use super::invoke_json;

    #[test]
    fn thin_raw_abi_delegates_to_the_runtime_owned_session() {
        let response = invoke_json(
            r#"{"op":"start","sessionId":"raw/smoke","sourceId":"smoke.hal","source":"(+ 1 2)"}"#,
        );
        let response = std::str::from_utf8(&response).unwrap();
        assert!(response.contains("\"ok\":true"));
        assert!(response.contains("\"handle\":1"));
    }
}
