/* SPDX-License-Identifier: MIT */
#include "bts2_app_inc.h"

/* The pinned SDK opens its audio client on START, before buffered packets set
 * play_state. Its stop_audio_playback() only drains the worker and closes the
 * client when play_state is true. START followed quickly by SUSPEND therefore
 * leaks a silent output client, which can resume after HFP capture ends.
 *
 * Enter the SDK's complete stop path even before buffering finishes. These
 * hooks run in its Bluetooth task, not our UI task. The decoder processes an
 * already queued START before STOP and acknowledges STOP before the SDK frees
 * anything. Both audio_close(NULL) and sifli_resample_close(NULL) are safe, so
 * this also covers a stream stopped before its worker has opened the client.
 * The original handler resets play_state and releases all decoder resources.
 */
void __real_bt_avsnk_hdl_suspend_ind(bts2s_av_inst_data *inst, uint8_t index);
void __wrap_bt_avsnk_hdl_suspend_ind(bts2s_av_inst_data *inst, uint8_t index)
{
    inst->con[index].snk_data.play_state = TRUE;
    __real_bt_avsnk_hdl_suspend_ind(inst, index);
}

void __real_bt_avsnk_close_handler(bts2s_av_inst_data *inst, uint8_t index);
void __wrap_bt_avsnk_close_handler(bts2s_av_inst_data *inst, uint8_t index)
{
    inst->con[index].snk_data.play_state = TRUE;
    __real_bt_avsnk_close_handler(inst, index);
}

void __real_bt_avsnk_abort_handler(bts2s_av_inst_data *inst, uint8_t index);
void __wrap_bt_avsnk_abort_handler(bts2s_av_inst_data *inst, uint8_t index)
{
    inst->con[index].snk_data.play_state = TRUE;
    __real_bt_avsnk_abort_handler(inst, index);
}

void __real_bt_avsnk_hdl_disc_handler(bts2s_av_inst_data *inst, uint8_t index);
void __wrap_bt_avsnk_hdl_disc_handler(bts2s_av_inst_data *inst, uint8_t index)
{
    inst->con[index].snk_data.play_state = TRUE;
    __real_bt_avsnk_hdl_disc_handler(inst, index);
}
