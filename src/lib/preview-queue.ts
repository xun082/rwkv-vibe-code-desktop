// At most one DOM patch in flight. New snapshots replace stale pending work;
// acknowledgements from an older document/sequence cannot unlock the queue.
export class PreviewQueue {
  private ready = false;
  private sequence = 0;
  private sent: string | null = null;
  private inFlight: number | null = null;
  markReady() { this.ready = true; }
  reset() { this.ready = false; this.sent = null; this.inFlight = null; }
  take(html: string) {
    if (!this.ready || this.inFlight !== null || this.sent === html) return null;
    this.sent = html;
    this.inFlight = ++this.sequence;
    return { html, sequence: this.sequence };
  }
  acknowledge(sequence: number) {
    if (this.inFlight !== sequence) return false;
    this.inFlight = null;
    return true;
  }
  reject(sequence: number) {
    if (this.acknowledge(sequence)) this.sent = null;
  }
}
