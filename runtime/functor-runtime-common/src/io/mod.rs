pub async fn load_bytes_async(path: &str) -> Result<Vec<u8>, String> {
    #[cfg(target_arch = "wasm32")]
    {
        use js_sys::Uint8Array;
        use wasm_bindgen::JsCast;
        use wasm_bindgen_futures::JsFuture;
        use web_sys::{Request, RequestInit, Response};

        let mut opts = RequestInit::new();
        opts.method("GET");

        let request = Request::new_with_str_and_init(path, &opts)
            .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".into()))?;
        let window = web_sys::window().ok_or("No global `window` exists")?;
        let response_value = JsFuture::from(window.fetch_with_request(&request))
            .await
            .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".into()))?;
        let response: Response = response_value
            .dyn_into()
            .map_err(|_| "Failed to cast to Response")?;

        // A missing asset must FAIL (like the native `File::open` path), not
        // return the 404 page's body as if it were the asset — otherwise the
        // glTF/PNG parser downstream chokes on that HTML and panics. Returning
        // Err here routes into the asset system's fallback (empty) asset instead.
        if !response.ok() {
            return Err(format!("{}: HTTP {}", path, response.status()));
        }

        let array_buffer = JsFuture::from(
            response
                .array_buffer()
                .map_err(|_| "Couldn't convert response to array buffer")?,
        )
        .await
        .map_err(|_| "Failed to convert to array buffer")?;
        let array = Uint8Array::new(&array_buffer);
        Ok(array.to_vec())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use tokio::fs::File;
        use tokio::io::AsyncReadExt;

        let mut file = File::open(path).await.map_err(|e| e.to_string())?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)
            .await
            .map_err(|e| e.to_string())?;
        Ok(buffer)
    }
}

pub async fn load_bytes_async2(path: String) -> Result<Vec<u8>, String> {
    #[cfg(target_arch = "wasm32")]
    {
        load_bytes_async(&path)
            .await
            .map_err(|e| format!("{}: {}", path, e))
    }
    // AssetHandle polls asset futures manually with a noop waker, once per
    // frame on the render thread. A chunked tokio::fs read only advances one
    // chunk per poll under that scheme, so large assets took minutes to load
    // natively (a 47MB glb ≈ thousands of frames). Until assets are driven by
    // a real executor (see docs/todo.md "async inbox"), read synchronously:
    // one frame hitch instead of a minutes-long stall.
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::read(&path).map_err(|e| format!("{}: {}", path, e))
    }
}
