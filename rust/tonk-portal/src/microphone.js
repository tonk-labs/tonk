// Runs in the trusted parent of each registered portal. Requests from nested
// guests relay until reaching the real page; the sandbox is never relaxed.
const owners = new WeakMap();
let active = null;
const fail = (message, name = 'NotAllowedError') => Object.assign(new Error(message), { name });
function send(port, type, id, extra = {}) { port.postMessage({ v: 1, type, id, ...extra }); }

function consent(signal, video, audio, seconds) {
  return new Promise((resolve, reject) => {
    const dialog = document.createElement('dialog');
    dialog.style.cssText = 'color:#e7ece3;background:#202620;border:1px solid #607057;border-radius:10px;padding:24px;max-width:380px;font:15px/1.5 system-ui';
    const heading = document.createElement('h2'); heading.textContent = video ? (audio ? 'Record camera and microphone?' : 'Record camera video?') : 'Record microphone audio?'; heading.style.fontSize = '20px';
    const description = document.createElement('p'); description.textContent = `This Tonk view will receive the recording. Recording stops after ${seconds} seconds or when you leave the view.`;
    const start = document.createElement('button'); start.textContent = 'Allow recording';
    const cancel = document.createElement('button'); cancel.textContent = 'Cancel';
    for (const b of [start, cancel]) b.style.cssText = 'padding:9px 14px;margin:8px 8px 0 0;cursor:pointer';
    dialog.append(heading, description, start, cancel);
    const end = (error) => { signal.removeEventListener('abort', abort); dialog.remove(); error ? reject(error) : resolve(); };
    const abort = () => end(fail('Recording cancelled.', 'AbortError'));
    start.onclick = () => end(); cancel.onclick = abort;
    dialog.oncancel = e => { e.preventDefault(); abort(); };
    signal.addEventListener('abort', abort, { once: true });
    if (signal.aborted) return abort();
    document.body.append(dialog); dialog.showModal(); start.focus();
  });
}

async function capture(session, port) {
  const { id, controller, seconds, video, audio } = session, signal = controller.signal;
  let stream, recorder, context, source, timer, meter, indicator;
  const release = () => {
    clearTimeout(timer); clearInterval(meter);
    source?.disconnect(); stream?.getTracks().forEach(t => t.stop());
    if (context && context.state !== 'closed') context.close().catch(() => {});
    indicator?.remove();
  };
  session.cleanup = release;
  try {
    const method = video ? 'recordVideo' : 'recordAudio';
    if (window.tonk) {
      if (typeof window.tonk[method] !== 'function')
        throw fail('The parent does not support this recording type.', 'NotSupportedError');
      const recording = await window.tonk[method]({ maxDurationSeconds: seconds, signal, audio,
        onLevel: value => send(port, 'mic-level', id, { value }) });
      session.stop = () => recording.stop();
      send(port, 'mic-started', id);
      const blob = await recording.result;
      if (!signal.aborted) send(port, 'mic-result', id, { blob });
      return;
    }
    if (!navigator.mediaDevices?.getUserMedia || !globalThis.MediaRecorder)
      throw fail('Media recording is not supported in this browser.', 'NotSupportedError');
    if (active) throw fail('Another Tonk view is already recording.', 'InvalidStateError');
    active = session;
    await consent(signal, video, audio, seconds);
    stream = await navigator.mediaDevices.getUserMedia({ audio: audio ? { echoCancellation: false, noiseSuppression: false, autoGainControl: false } : false, video: video ? { width: { ideal: 1280, max: 1280 }, height: { ideal: 720, max: 720 }, frameRate: { ideal: 30, max: 30 } } : false });
    if (signal.aborted) throw fail('Recording cancelled.', 'AbortError');
    const mimeType = (video ? ['video/webm;codecs=vp8,opus', 'video/mp4', 'video/webm'] : ['audio/webm;codecs=opus', 'audio/mp4', 'audio/webm']).find(t => MediaRecorder.isTypeSupported(t));
    recorder = new MediaRecorder(stream, { ...(mimeType ? { mimeType } : {}), audioBitsPerSecond: 64000, ...(video ? { videoBitsPerSecond: 2500000 } : {}) });
    const chunks = [];
    const blobPromise = new Promise((resolve, reject) => {
      recorder.ondataavailable = e => { if (e.data.size) chunks.push(e.data); };
      recorder.onerror = e => reject(e.error || fail('Media recording failed.', 'NotReadableError'));
      recorder.onstop = () => resolve(new Blob(chunks, { type: recorder.mimeType }));
    });
    session.stop = () => { if (recorder.state === 'recording') recorder.stop(); };
    const abort = () => { session.stop(); release(); };
    signal.addEventListener('abort', abort, { once: true });
    indicator = document.createElement('div');
    indicator.style.cssText = 'position:fixed;top:12px;right:12px;z-index:2147483647;background:#34241f;color:#ffdad0;border:1px solid #c77c65;border-radius:6px;padding:10px 14px;font:14px system-ui;display:flex;gap:12px;align-items:center';
    const label = document.createElement('span'); label.textContent = video ? (audio ? '● Camera and microphone recording' : '● Camera recording') : '● Microphone recording';
    const stop = document.createElement('button'); stop.textContent = 'Stop'; stop.onclick = session.stop;
    indicator.append(label, stop); document.body.append(indicator);
    recorder.start(100);
    timer = setTimeout(session.stop, seconds * 1000);
    send(port, 'mic-started', id);
    // The metering path is optional: a suspended AudioContext must not break capture.
    try {
      if (!audio) throw new Error('No audio requested');
      context = new AudioContext(); context.resume().catch(() => {});
      source = context.createMediaStreamSource(stream); const analyser = context.createAnalyser(); analyser.fftSize = 256; source.connect(analyser);
      const wave = new Uint8Array(128), start = performance.now();
      meter = setInterval(() => { analyser.getByteTimeDomainData(wave); send(port, 'mic-level', id, { value: { elapsed: (performance.now() - start) / 1000, waveform: Array.from(wave) } }); }, 50);
    } catch { /* Audio is still captured by MediaRecorder. */ }
    const blob = await blobPromise;
    signal.removeEventListener('abort', abort);
    if (!signal.aborted) send(port, 'mic-result', id, { blob });
  } catch (error) {
    if (!signal.aborted) send(port, 'mic-error', id, { error: error.message || String(error), name: error.name || 'Error' });
  } finally {
    release(); if (active === session) active = null;
    if (owners.get(port) === session) owners.delete(port);
  }
}

export function handleMicrophone(data, port) {
  if (!data || typeof data.id !== 'string' || data.id.length > 100) return;
  const existing = owners.get(port);
  if (data.type === 'mic-start') {
    if (existing) { send(port, 'mic-error', data.id, { error: 'A recording is already active for this view.', name: 'InvalidStateError' }); return; }
    const seconds = Math.max(1, Math.min(30, Number(data.seconds) || 30));
    const session = { id: data.id, seconds, video: data.video === true, audio: data.video !== true || data.audio !== false, controller: new AbortController() };
    owners.set(port, session); void capture(session, port);
  } else if (existing?.id === data.id) {
    if (data.type === 'mic-stop') existing.stop?.();
    if (data.type === 'mic-cancel') {
      existing.controller.abort(); existing.cleanup?.();
      send(port, 'mic-error', data.id, { error: 'Recording cancelled.', name: 'AbortError' });
    }
  }
}

export function disposeMicrophone(port) {
  const session = owners.get(port);
  if (session) { session.controller.abort(); session.cleanup?.(); owners.delete(port); }
}
