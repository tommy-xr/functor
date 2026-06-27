/** Error thrown when the debug runtime returns a non-2xx response. */
export class HttpError extends Error {
  constructor(
    public readonly method: string,
    public readonly url: string,
    public readonly status: number,
    public readonly body: string,
  ) {
    super(`${method} ${url} failed with status ${status}: ${body}`);
    this.name = "HttpError";
  }
}

/** Thin typed wrapper over fetch for the debug runtime's API.
 *
 * The functor debug runtime is not uniformly JSON: `/state` and `/scene` return
 * JSON, `/input` and `/time` return the literal text `ok`, and `/capture`
 * returns binary PNG. This client exposes one method per response shape. */
export class HttpClient {
  /** @param timeoutMs per-request timeout; prevents a stuck connection (TCP
   * accepted but no response) from hanging readiness polling forever. */
  constructor(
    public readonly baseUrl: string,
    private readonly timeoutMs = 10_000,
  ) {}

  /** GET a JSON body. */
  async getJson<T>(path: string): Promise<T> {
    const res = await this.send("GET", path);
    return JSON.parse(await res.text()) as T;
  }

  /** POST a JSON body to an endpoint that replies with plain text (e.g. `ok`). */
  async postText(path: string, body?: unknown): Promise<string> {
    const res = await this.send("POST", path, body);
    return res.text();
  }

  /** POST a JSON body to an endpoint that replies with binary (e.g. a PNG). */
  async postBinary(path: string, body?: unknown): Promise<Buffer> {
    const res = await this.send("POST", path, body);
    return Buffer.from(await res.arrayBuffer());
  }

  private async send(
    method: string,
    path: string,
    body?: unknown,
  ): Promise<Response> {
    const url = `${this.baseUrl}${path}`;
    const response = await fetch(url, {
      method,
      headers:
        body !== undefined ? { "Content-Type": "application/json" } : undefined,
      body: body !== undefined ? JSON.stringify(body) : undefined,
      signal: AbortSignal.timeout(this.timeoutMs),
    });
    if (!response.ok) {
      throw new HttpError(method, url, response.status, await response.text());
    }
    return response;
  }
}
