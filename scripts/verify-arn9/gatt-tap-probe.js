/**
 * gatt-tap-probe.js — 只读 Frida 探针：抓 WUDFHost 里的 GATT 特征读取
 *
 * 目的：按遥控器的 返回 / 音量+ / 音量-，验证能否在 NtDeviceIoControlFile
 *       的 IOCTL 0x80018483（HID-over-GATT ReadCharacteristic）返回缓冲里
 *       抓到 9 字节报告中的 F1 / 80 / 81。
 *
 * 用法（管理员 PowerShell，先跑 device-probe.ps1 拿到 HostPid）：
 *   pip install frida-tools
 *   frida -p <HostPid> -l scripts\verify-arn9\gatt-tap-probe.js
 *
 * 在 frida REPL 里看输出；验证完 Ctrl+C / %q 退出。
 *
 * 判读：
 *   PASS —— 按下三个键时出现类似
 *            [HIT] IOCTL=0x80018483 len=9 data=01 00 00 F1 00 80 00 81 00
 *            （F1=返回, 80=音量+, 81=音量-，同时按会拼在同一报告里）
 *   FAIL —— 三个键完全无输出，但方向/OK 键有输出：
 *            说明这三个键根本不走 GATT 读通道 → 方案要重新评估。
 *   FAIL —— 所有键都无输出：注入目标 PID 不对，回第 1 步核对。
 *
 * 本脚本只读不写，不改任何键的行为。
 */

const READ_CHARACTERISTIC_IOCTL = 0x80018483;
const EXPECTED_OUTPUT_LENGTH = 9;

// 只打印目标 IOCTL，或长度为 9 的读结果（防其它 IOCTL 刷屏）
function shouldLog(ioctl, outputLength) {
    return ioctl === READ_CHARACTERISTIC_IOCTL
        || outputLength === EXPECTED_OUTPUT_LENGTH;
}

function hex(arrayBuffer) {
    const bytes = new Uint8Array(arrayBuffer);
    let out = '';
    for (let i = 0; i < bytes.length; i++) {
        out += bytes[i].toString(16).padStart(2, '0').toUpperCase();
        if (i < bytes.length - 1) out += ' ';
    }
    return out;
}

function annotate(data) {
    const bytes = new Uint8Array(data);
    const marks = [];
    if (bytes.length >= 7 && bytes[3] === 0xf1 && bytes[4] === 0x00) marks.push('BACK(0xF1)');
    if (bytes.length >= 7 && bytes[5] === 0x80 && bytes[6] === 0x00) marks.push('VOL+(0x80)');
    if (bytes.length >= 9 && bytes[7] === 0x81 && bytes[8] === 0x00) marks.push('VOL-(0x81)');
    // 通用：任意位置出现 F1/80/81 + 00 的 little-endian usage
    for (let i = 0; i + 1 < bytes.length; i++) {
        const usage = bytes[i] | (bytes[i + 1] << 8);
        const name = usage === 0x00f1 ? 'BACK'
            : usage === 0x0080 ? 'VOL+'
            : usage === 0x0081 ? 'VOL-'
            : null;
        if (name && !marks.some(m => m.startsWith(name))) marks.push(name + '(0x' + usage.toString(16).toUpperCase() + ')');
    }
    return marks.length ? '  << ' + marks.join(' ') : '';
}

function installHook() {
    const ntdll = Process.findModuleByName('ntdll.dll');
    if (!ntdll) {
        console.log('[-] ntdll.dll not found');
        return;
    }
    const target = ntdll.findExportByName('NtDeviceIoControlFile');
    if (!target) {
        console.log('[-] NtDeviceIoControlFile export not found');
        return;
    }

    Interceptor.attach(target, {
        onEnter(args) {
            this.ioctl = args[5].toUInt32();
            this.output = args[8];
            this.outputLength = args[9].toUInt32();
            this.capture = shouldLog(this.ioctl, this.outputLength)
                && !this.output.isNull()
                && this.outputLength > 0
                && this.outputLength <= 64;
        },
        onLeave(retval) {
            if (!this.capture) return;
            if (retval.toUInt32() !== 0) return; // STATUS_SUCCESS only
            let data;
            try {
                data = this.output.readByteArray(this.outputLength);
            } catch (e) {
                return;
            }
            if (data === null) return;
            console.log('[HIT] IOCTL=0x' + this.ioctl.toString(16)
                + ' len=' + this.outputLength
                + ' data=' + hex(data)
                + annotate(data));
        },
    });

    console.log('[*] Hooked NtDeviceIoControlFile @ ' + target);
    console.log('[*] Watching IOCTL 0x' + READ_CHARACTERISTIC_IOCTL.toString(16)
        + ' and any ' + EXPECTED_OUTPUT_LENGTH + '-byte reads.');
    console.log('[*] Press BACK / VOL+ / VOL- on the remote now (also try OK/direction as control).');
}

installHook();
