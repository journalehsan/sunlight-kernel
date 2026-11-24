static void capture_ack(unsigned sequence)
{
    static const char *const waiting[] = {
        "[WL-CAPTURE] WAIT_FRAME_1_CAPTURE_ACK\n",
        "[WL-CAPTURE] WAIT_FRAME_2_CAPTURE_ACK\n",
        "[WL-CAPTURE] WAIT_SGP_REMOVAL\n",
    };
    static const char *const received[] = {
        "[WL-CAPTURE] frame-1-ack-received\n",
        "[WL-CAPTURE] frame-2-ack-received\n",
        "[WL-CAPTURE] removal-ack-received\n",
    };
    static const char *const timeout[] = {
        "[WL-CLIENT] FAIL capture-ack timeout frame=1\n",
        "[WL-CLIENT] FAIL capture-ack timeout frame=2\n",
        "[WL-CLIENT] FAIL SGP removal acknowledgement timeout\n",
    };
    capture_log(waiting[sequence - 1]);
    long start = capture_native(86, 0, 0);
    while (capture_native(86, 0, 0) - start < 25000) {
        long byte = capture_native(0x574c, 0, 0);
        if (byte == '0' + sequence) {
            capture_log(received[sequence - 1]);
            return;
        }
        require(byte == 0, "[WL-CLIENT] FAIL unexpected capture acknowledgement\n");
        capture_pause();
    }
    require(0, timeout[sequence - 1]);
}
