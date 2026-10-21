(async () => {
  const text = "Élève français, cœur, où êtes-vous ?";
  const canvas = document.createElement("canvas");
  canvas.width = 300; canvas.height = 70;
  const ctx = canvas.getContext("2d");
  ctx.fillStyle = "#e7d8aa"; ctx.fillRect(0, 0, 300, 70);
  ctx.fillStyle = "#263655"; ctx.font = "18px Arial"; ctx.fillText(text, 5, 30);
  const metrics = {};
  for (const font of ["Arial", "Times New Roman", "Courier New", "Helvetica", "Menlo", "PingFang SC", "Microsoft YaHei"]) {
    ctx.font = `18px "${font}", monospace`;
    metrics[font] = ctx.measureText(text).width;
  }
  const audio = new OfflineAudioContext(1, 10000, 44100);
  const oscillator = audio.createOscillator();
  oscillator.type = "triangle"; oscillator.frequency.value = 1000;
  const compressor = audio.createDynamicsCompressor();
  oscillator.connect(compressor); compressor.connect(audio.destination); oscillator.start();
  const buffer = await audio.startRendering();
  const samples = Array.from(buffer.getChannelData(0).slice(4000, 4100));
  const gl = document.createElement("canvas").getContext("webgl");
  const info = gl && gl.getExtension("WEBGL_debug_renderer_info");
  return {
    language: navigator.language, languages: navigator.languages,
    timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    userAgent: navigator.userAgent, platform: navigator.platform,
    hardwareConcurrency: navigator.hardwareConcurrency,
    devicePixelRatio: window.devicePixelRatio,
    screen: [screen.width, screen.height, screen.availWidth, screen.availHeight, screen.colorDepth, screen.pixelDepth, screen.availLeft, screen.availTop],
    canvas: canvas.toDataURL(), metrics, audio: samples,
    webgl: info ? [gl.getParameter(info.UNMASKED_VENDOR_WEBGL), gl.getParameter(info.UNMASKED_RENDERER_WEBGL)] : null,
  };
})()
