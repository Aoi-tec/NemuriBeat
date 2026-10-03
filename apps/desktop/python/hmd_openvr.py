"""Read-only OpenVR pose/proximity worker; never starts a VR rendering application."""
import json
import sys
import time

def emit(value):
    print(json.dumps(value, allow_nan=False), flush=True)

def worn_state(vr, system):
    try:
        if not system.getBoolTrackedDeviceProperty(vr.k_unTrackedDeviceIndex_Hmd, vr.Prop_ContainsProximitySensor_Bool):
            return None
        ok, state = system.getControllerState(vr.k_unTrackedDeviceIndex_Hmd)
        if ok:
            return bool(state.ulButtonPressed & (1 << vr.k_EButton_ProximitySensor))
    except Exception:
        pass
    # ActivityLevel mixes movement and proximity, so it is not used as a worn flag.
    return None

def main():
    try:
        import openvr as vr
    except Exception as exc:
        emit({'available': False, 'message': 'OpenVRを読み込めません: ' + type(exc).__name__})
        return 1
    while True:
        initialized = False
        try:
            if not vr.isHmdPresent():
                raise RuntimeError('HMD not present')
            system = vr.init(vr.VRApplication_Background)
            initialized = True
            poses = (vr.TrackedDevicePose_t * vr.k_unMaxTrackedDeviceCount)()
            while True:
                begin = time.monotonic()
                system.getDeviceToAbsoluteTrackingPose(vr.TrackingUniverseStanding, 0, poses)
                pose = poses[vr.k_unTrackedDeviceIndex_Hmd]
                valid = pose.bDeviceIsConnected and pose.bPoseIsValid and pose.eTrackingResult == vr.TrackingResult_Running_OK
                emit({'timestamp': time.time_ns() // 1000000, 'available': bool(valid),
                      'velocity': list(pose.vVelocity.v) if valid else [0,0,0],
                      'angular_velocity': list(pose.vAngularVelocity.v) if valid else [0,0,0],
                      'worn': worn_state(vr, system) if valid else None,
                      'message': 'OpenVR pose受信中' if valid else 'HMDの追跡が無効です'})
                time.sleep(max(0, .1 - (time.monotonic() - begin)))
        except (BrokenPipeError, KeyboardInterrupt):
            return 0
        except Exception as exc:
            emit({'available': False, 'message': 'OpenVR待機中: ' + type(exc).__name__})
        finally:
            if initialized:
                vr.shutdown()
        time.sleep(5)

if __name__ == '__main__':
    sys.exit(main())
