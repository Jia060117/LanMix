package io.lanmix.receiver;

import java.net.DatagramSocket;
import java.net.DatagramPacket;
import java.net.InetAddress;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.Arrays;
import java.util.concurrent.atomic.AtomicReference;

/** Runs the real host DLL through the same JNI signatures under -Xcheck:jni. */
public final class NativeBridge {
    public native void nativeStart(int port, int bufferMs, int rate);
    public native void nativeStop();
    public native void nativeFill(float[] output);
    public native String nativeTracks();
    public native String nativeSources();
    public native void nativeSetTrack(int index, float gain, boolean mute, boolean solo,
        boolean enabled, float preamp, float[] bands);
    public native void nativeSetMaster(float gain, boolean enabled, float preamp, float[] bands);
    public native void nativeDiscoveryUpdate(String identity);
    public native String nativeDiscoverySnapshot();
    public native void nativeDiscoveryConnect(String id);
    public native void nativeDiscoveryProbe(String address);
    public native void nativeDiscoveryStop();

    static void discovery(NativeBridge bridge) throws Exception {
        String receiverId = "11111111111111111111111111111111";
        String senderId = "22222222222222222222222222222222";
        String identity = "{\"id\":\"" + receiverId + "\",\"name\":\"JNI receiver\",\"role\":\"receiver\","
            + "\"audio_port\":46210,\"ready\":true,\"version\":\"test\"}";
        String hello = "{\"magic\":\"LMXD\",\"protocol\":1,\"kind\":\"hello\",\"peer\":{\"id\":\"" + senderId
            + "\",\"name\":\"JNI sender\",\"role\":\"sender\",\"audio_port\":0,\"ready\":true,\"version\":\"test\"}}";
        rejected(() -> bridge.nativeDiscoveryUpdate("{}"));
        bridge.nativeDiscoveryUpdate(identity);
        try (DatagramSocket socket = new DatagramSocket(0, InetAddress.getLoopbackAddress())) {
            socket.setSoTimeout(2000);
            rejected(() -> bridge.nativeDiscoveryProbe("not-an-ip"));
            bridge.nativeDiscoveryProbe("127.0.0.1:" + socket.getLocalPort());
            DatagramPacket incoming = new DatagramPacket(new byte[2048], 2048);
            socket.receive(incoming);
            String probe = new String(incoming.getData(), 0, incoming.getLength(), java.nio.charset.StandardCharsets.UTF_8);
            require(probe.contains("\"kind\":\"probe\"") && probe.contains(receiverId), "JNI direct discovery probe");
            byte[] bytes = hello.getBytes(java.nio.charset.StandardCharsets.UTF_8);
            socket.send(new DatagramPacket(bytes, bytes.length, incoming.getAddress(), incoming.getPort()));
            long deadline = System.nanoTime() + 2_000_000_000L;
            while (!bridge.nativeDiscoverySnapshot().contains(senderId)) {
                require(System.nanoTime() < deadline, "JNI discovery peer timeout"); Thread.sleep(20);
            }
            bridge.nativeDiscoveryUpdate(identity); // update the live worker, no restart
            bridge.nativeDiscoveryConnect(senderId);
            boolean connected = false;
            while (!connected && System.nanoTime() < deadline) {
                incoming.setLength(2048); socket.receive(incoming);
                String message = new String(incoming.getData(), 0, incoming.getLength(), java.nio.charset.StandardCharsets.UTF_8);
                connected = message.contains("\"kind\":\"connect\"") && message.contains("\"target\":\"" + senderId + "\"");
            }
            require(connected, "JNI receiver-to-sender connect request");
            String session = hello.substring(0, hello.length() - 1).replace("\"kind\":\"hello\"", "\"kind\":\"session\"")
                + ",\"target\":\"" + receiverId + "\",\"session\":{\"state\":\"connected\",\"stream_id\":1,\"revision\":1}}";
            bytes = session.getBytes(java.nio.charset.StandardCharsets.UTF_8);
            socket.send(new DatagramPacket(bytes, bytes.length, incoming.getAddress(), incoming.getPort()));
            deadline = System.nanoTime() + 2_000_000_000L;
            while (!bridge.nativeDiscoverySnapshot().contains("\"state\":\"connected\"")) {
                require(System.nanoTime() < deadline, "silent session crosses JNI without PCM"); Thread.sleep(20);
            }
            require(bridge.nativeSources().equals("[]"), "session does not fabricate an audio source");
            bytes = session.replace("\"connected\"", "\"disconnected\"").replace("\"revision\":1", "\"revision\":2")
                .getBytes(java.nio.charset.StandardCharsets.UTF_8);
            socket.send(new DatagramPacket(bytes, bytes.length, incoming.getAddress(), incoming.getPort()));
            deadline = System.nanoTime() + 2_000_000_000L;
            while (!bridge.nativeDiscoverySnapshot().contains("\"sessions\":[]")) {
                require(System.nanoTime() < deadline, "silent disconnect crosses JNI"); Thread.sleep(20);
            }
            bytes = session.getBytes(java.nio.charset.StandardCharsets.UTF_8);
            socket.send(new DatagramPacket(bytes, bytes.length, incoming.getAddress(), incoming.getPort()));
            Thread.sleep(100);
            require(bridge.nativeDiscoverySnapshot().contains("\"sessions\":[]"), "late session cannot revive disconnected UI");
        } finally { bridge.nativeDiscoveryStop(); }
        require(bridge.nativeDiscoverySnapshot().contains("\"peers\":[]"), "discovery worker released");
        System.out.println("JNI discovery: identity/update, UDP probe/reply, connect, silent session/disconnect, revision ordering and stop passed");
    }

    static void require(boolean condition, String message) {
        if (!condition) throw new AssertionError(message);
    }
    static void rejected(Runnable action) {
        try { action.run(); } catch (IllegalStateException expected) { return; }
        throw new AssertionError("Expected JNI argument/lifecycle exception");
    }
    static byte[] packet(int sequence) {
        ByteBuffer bytes = ByteBuffer.allocate(40 + 140 * 8).order(ByteOrder.LITTLE_ENDIAN);
        bytes.put((byte)'L').put((byte)'M').put((byte)1).put((byte)0);
        bytes.putLong(0x11223344L).putInt(7).putInt(sequence).putLong(sequence * 140L);
        bytes.putInt(48000).putShort((short)140).put((byte)2).put((byte)1).putInt(0);
        for (int i = 0; i < 140; i++) bytes.putFloat(.2f).putFloat(-.1f);
        return bytes.array();
    }
    static void send(DatagramSocket socket, int port, int from, int count) throws Exception {
        for (int i = from; i < from + count; i++) {
            byte[] bytes = packet(i);
            socket.send(new DatagramPacket(bytes, bytes.length, InetAddress.getLoopbackAddress(), port));
        }
        Thread.sleep(100);
    }

    static void sourceAdmission(NativeBridge bridge) throws Exception {
        int port;
        try (DatagramSocket reservation = new DatagramSocket(0)) { port = reservation.getLocalPort(); }
        bridge.nativeStart(port, 150, 48000);
        try (DatagramSocket sender = new DatagramSocket()) {
            float[] output = new float[960];
            bridge.nativeFill(output);
            bridge.nativeSetMaster(0f, false, 0f, new float[10]);
            long start = System.nanoTime(), nextPacket = start, nextFill = start, nextSources = start;
            long sourceMs = -1, soundMs = -1;
            int sequence = 0;
            while (System.nanoTime() - start < 2_000_000_000L && soundMs < 0) {
                long now = System.nanoTime();
                if (now >= nextPacket) {
                    byte[] bytes = packet(sequence++);
                    sender.send(new DatagramPacket(bytes, bytes.length, InetAddress.getLoopbackAddress(), port));
                    nextPacket += 3_000_000L;
                }
                if (now >= nextSources) {
                    String sources = bridge.nativeSources();
                    require(!sources.contains("meters") && !sources.contains("packets"), "lightweight topology excludes diagnostics");
                    if (sourceMs < 0 && sources.contains("\"stream_id\":7")) {
                        require(sources.contains("\"source\":\"127.0.0.1\"") && sources.contains("\"sample_rate\":48000"), "source identity/format cross JNI");
                        bridge.nativeSetTrack(0, -3f, false, false, false, 0f, new float[10]);
                        sourceMs = (System.nanoTime() - start) / 1_000_000L;
                    }
                    nextSources = now + 50_000_000L;
                }
                if (now >= nextFill) {
                    bridge.nativeFill(output);
                    for (float sample : output) {
                        require(Float.isFinite(sample), "admission finite PCM");
                        if (Math.abs(sample) > .001f) { soundMs = (System.nanoTime() - start) / 1_000_000L; break; }
                    }
                    nextFill = now + 10_000_000L;
                }
                Thread.sleep(1);
            }
            require(sourceMs >= 0 && soundMs >= sourceMs, "source admission opens saved controls without any nativeTracks call");
            System.out.println("JNI source admission: source=" + sourceMs + " ms, rendered sound=" + soundMs
                + " ms at 150 ms network buffer; no diagnostic reads (host simulation, not device latency)");
        } finally { bridge.nativeStop(); }
        require(bridge.nativeSources().equals("[]"), "stopped source topology");
    }
    public static void main(String[] args) throws Exception {
        System.load(args[0]);
        NativeBridge bridge = new NativeBridge();
        discovery(bridge);
        sourceAdmission(bridge);
        bridge.nativeStop(); bridge.nativeStop();
        require(bridge.nativeTracks().contains("\"running\":false"), "stopped JSON");
        rejected(() -> bridge.nativeStart(0, 60, 48000));
        rejected(() -> bridge.nativeFill(new float[2048]));
        int port;
        try (DatagramSocket reservation = new DatagramSocket(0)) { port = reservation.getLocalPort(); }
        bridge.nativeStart(port, 60, 48000);
        try (DatagramSocket sender = new DatagramSocket()) {
            bridge.nativeSetMaster(-6f, true, -3f, new float[10]);
            float[] output = new float[480 * 2]; // current 10 ms Android write block
            Arrays.fill(output, Float.NaN);
            bridge.nativeFill(output); // first fill claims worker TLS before packets
            require(Arrays.equals(output, new float[480 * 2]), "fill overwrites all Java samples");
            rejected(() -> bridge.nativeFill(new float[3]));
            rejected(() -> bridge.nativeFill(new float[8194]));
            rejected(() -> bridge.nativeSetMaster(0f, false, 0f, new float[9]));
            rejected(() -> bridge.nativeSetTrack(16, 0f, false, false, false, 0f, new float[10]));
            send(sender, port, 0, 50);
            String snapshot = bridge.nativeTracks();
            require(snapshot.contains("\"stream_id\":7") && snapshot.contains("\"source\":\"127.0.0.1\""), "real UDP snapshot");
            require(snapshot.contains("\"target_buffer_ms\":") && snapshot.contains("\"buffered_ms\":")
                && snapshot.contains("\"primed\":") && snapshot.contains("\"drift_ppm\":"), "buffer diagnostics cross JNI");
            require(snapshot.contains("\"reset_count\":") && snapshot.contains("\"reset_reason\":\"new_source\"")
                && snapshot.contains("\"concealed_frames\":") && snapshot.contains("\"loss_events\":"), "reset/loss diagnostics cross JNI");
            bridge.nativeFill(output); // process receiver lifecycle/reset while gated
            send(sender, port, 50, 50);
            bridge.nativeFill(output);
            require(Arrays.equals(output, new float[480 * 2]), "unknown identity starts muted");
            bridge.nativeSetTrack(0, -3f, false, true, true, -2f, new float[10]);
            send(sender, port, 100, 50);
            bridge.nativeFill(output);
            double peak = 0;
            for (float value : output) { require(Float.isFinite(value), "finite PCM"); peak = Math.max(peak, Math.abs(value)); }
            require(peak > .001 && peak <= .98, "identified source becomes audible through JNI");

            AtomicReference<Throwable> failure = new AtomicReference<>();
            Thread wrongWorker = new Thread(() -> {
                try { rejected(() -> bridge.nativeFill(new float[2048])); }
                catch (Throwable e) { failure.set(e); }
            });
            wrongWorker.start(); wrongWorker.join();
            if (failure.get() != null) throw new AssertionError("worker ownership", failure.get());
            Thread ui = new Thread(() -> {
                try {
                    for (int i = 0; i < 100; i++) {
                        require(bridge.nativeTracks().contains("\"running\":true"), "running JSON");
                        require(bridge.nativeSources().contains("\"stream_id\":7"), "concurrent source topology");
                        bridge.nativeSetMaster(-6f, i % 2 == 0, 0f, new float[10]);
                    }
                } catch (Throwable e) { failure.set(e); }
            });
            ui.start();
            for (int i = 0; i < 100; i++) bridge.nativeFill(output);
            ui.join();
            if (failure.get() != null) throw new AssertionError("concurrent UI controls/diagnostics", failure.get());
            System.out.println("JNI real UDP gated->audible peak=" + peak + "; concurrent controls/diagnostics and TLS ownership passed");
        } finally { bridge.nativeStop(); }
        bridge.nativeStart(port, 60, 96000);
        bridge.nativeFill(new float[8192]); // maximum size; new epoch on same JVM worker
        bridge.nativeStop();
        bridge.nativeStart(port, 60, 48000);
        try (DatagramSocket sender = new DatagramSocket()) {
            float[] block = new float[480 * 2];
            bridge.nativeFill(block);
            send(sender, port, 0, 1);
            bridge.nativeFill(block); // acknowledge first source reset before supplying the test audio
            bridge.nativeSetMaster(0f, false, 0f, new float[10]);
            bridge.nativeSetTrack(0, 0f, false, false, false, 0f, new float[10]);
            send(sender, port, 1, 50);
            send(sender, port, 56, 50); // sequences 51..55: 700 genuinely missing frames
            boolean heard = false;
            int quietFramesAfterSound = 0;
            for (int i = 0; i < 25; i++) {
                bridge.nativeFill(block);
                for (int sample = 0; sample < block.length; sample += 2) {
                    if (Math.abs(block[sample]) > .01f) heard = true;
                    if (heard && Math.abs(block[sample]) < 1e-6f && Math.abs(block[sample + 1]) < 1e-6f)
                        quietFramesAfterSound++;
                }
            }
            String snapshot = bridge.nativeTracks();
            require(heard && quietFramesAfterSound > 200, "real missing-packet concealment produces a PCM dip");
            require(snapshot.contains("\"concealed_frames\":700") && snapshot.contains("\"loss_events\":1"), "lifetime missing-frame diagnostics");
            require(snapshot.contains("\"missing_datagrams\":5") && snapshot.contains("\"transport_gap_frames\":700")
                && snapshot.contains("\"sender_gap_frames\":0"), "sequence diagnostics identify unreceived datagrams");
            require(!java.util.regex.Pattern.compile("\"underruns\":[1-9]").matcher(snapshot).find(), "concealment is not mixer starvation");
            System.out.println("JNI real UDP loss: 700 missing frames, " + quietFramesAfterSound + " quiet frames after sound, mixer underruns remain zero");
            byte[] nextSession = packet(0);
            ByteBuffer.wrap(nextSession).order(ByteOrder.LITTLE_ENDIAN).putLong(4, 0x11223345L);
            sender.send(new DatagramPacket(nextSession, nextSession.length, InetAddress.getLoopbackAddress(), port));
            Thread.sleep(100);
            String afterReset = bridge.nativeTracks();
            require(afterReset.contains("\"concealed_frames\":700") && afterReset.contains("\"reset_count\":2")
                && afterReset.contains("\"reset_reason\":\"sender_session\""), "lifetime loss evidence survives a sender-session reset");
            require(afterReset.contains("\"missing_datagrams\":5"), "sequence evidence survives session reset");
        } finally { bridge.nativeStop(); }
        bridge.nativeStart(port, 150, 48000);
        try (DatagramSocket sender = new DatagramSocket()) {
            float[] block = new float[480 * 2];
            bridge.nativeFill(block);
            send(sender, port, 0, 1);
            bridge.nativeFill(block);
            bridge.nativeSetMaster(0f, false, 0f, new float[10]);
            bridge.nativeSetTrack(0, 0f, false, false, false, 0f, new float[10]);
            send(sender, port, 1, 50);
            // No 100 ms helper wait between the gap and its repair: recovery
            // must beat the actual 120 ms reorder deadline at buffer=150 ms.
            for (int sequence = 54; sequence < 130; sequence++) {
                byte[] bytes = packet(sequence);
                sender.send(new DatagramPacket(bytes, bytes.length, InetAddress.getLoopbackAddress(), port));
            }
            Thread.sleep(20);
            for (int sequence = 1; sequence < 130; sequence++) {
                byte[] bytes = packet(sequence);
                sender.send(new DatagramPacket(bytes, bytes.length, InetAddress.getLoopbackAddress(), port));
            }
            Thread.sleep(100);
            String repaired = bridge.nativeTracks();
            require(repaired.contains("\"gap_filled_packets\":3") && repaired.contains("\"duplicate_packets\":126")
                && repaired.contains("\"concealed_frames\":0") && repaired.contains("\"missing_datagrams\":0"), "real UDP repair and deduplication");
            // Enough audio remains before and after the repaired interval;
            // constant nonzero PCM must not acquire a 2 ms silent dropout.
            int quietRun = 0, maxQuiet = 0;
            boolean heard = false;
            for (int i = 0; i < 33; i++) {
                bridge.nativeFill(block);
                for (int sample = 0; sample < block.length; sample += 2) {
                    if (Math.abs(block[sample]) > .01f) heard = true;
                    if (heard && Math.abs(block[sample]) < 1e-6f && Math.abs(block[sample + 1]) < 1e-6f) quietRun++;
                    else quietRun = 0;
                    maxQuiet = Math.max(maxQuiet, quietRun);
                }
            }
            require(heard && maxQuiet < 96, "no 2 ms silent run after real UDP burst repair");
            System.out.println("JNI real UDP repair: 3 missing originals restored, 126 duplicates removed, max quiet run=" + maxQuiet);
        } finally { bridge.nativeStop(); }
        for (int rate : new int[] {8000, 44100, 48000, 96000, 192000}) {
            bridge.nativeStart(port, 150, rate);
            try {
                float[] block = new float[((rate + 99) / 100) * 2];
                Arrays.fill(block, Float.NaN);
                bridge.nativeFill(block);
                require(Arrays.equals(block, new float[block.length]), "10 ms fill at " + rate);
            } finally { bridge.nativeStop(); }
        }
        System.out.println("JNI lifecycle, array validation, JSON, all twelve exports, and 10 ms fills at 8/44.1/48/96/192 kHz passed");
    }
}
