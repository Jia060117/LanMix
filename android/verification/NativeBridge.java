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
    public native void nativeSetTrack(int index, float gain, boolean mute, boolean solo,
        boolean enabled, float preamp, float[] bands);
    public native void nativeSetMaster(float gain, boolean enabled, float preamp, float[] bands);

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
    public static void main(String[] args) throws Exception {
        System.load(args[0]);
        NativeBridge bridge = new NativeBridge();
        bridge.nativeStop(); bridge.nativeStop();
        require(bridge.nativeTracks().contains("\"running\":false"), "stopped JSON");
        rejected(() -> bridge.nativeStart(0, 60, 48000));
        rejected(() -> bridge.nativeFill(new float[2048]));
        int port;
        try (DatagramSocket reservation = new DatagramSocket(0)) { port = reservation.getLocalPort(); }
        bridge.nativeStart(port, 60, 48000);
        try (DatagramSocket sender = new DatagramSocket()) {
            bridge.nativeSetMaster(-6f, true, -3f, new float[10]);
            float[] output = new float[2048];
            Arrays.fill(output, Float.NaN);
            bridge.nativeFill(output); // first fill claims worker TLS before packets
            require(Arrays.equals(output, new float[2048]), "fill overwrites all Java samples");
            rejected(() -> bridge.nativeFill(new float[3]));
            rejected(() -> bridge.nativeFill(new float[8194]));
            rejected(() -> bridge.nativeSetMaster(0f, false, 0f, new float[9]));
            rejected(() -> bridge.nativeSetTrack(16, 0f, false, false, false, 0f, new float[10]));
            send(sender, port, 0, 50);
            String snapshot = bridge.nativeTracks();
            require(snapshot.contains("\"stream_id\":7") && snapshot.contains("\"source\":\"127.0.0.1\""), "real UDP snapshot");
            bridge.nativeFill(output); // process receiver lifecycle/reset while gated
            send(sender, port, 50, 50);
            bridge.nativeFill(output);
            require(Arrays.equals(output, new float[2048]), "unknown identity starts muted");
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
        System.out.println("JNI lifecycle, array validation, JSON, all six exports, and 48k->96k restart passed");
    }
}
