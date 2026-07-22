# CMake toolchain file that sets the policy version minimum.
# This fixes builds with CMake >= 3.27 which removed compatibility
# with cmake_minimum_required < 3.5. The bundled opus source in
# audiopus_sys uses an old cmake_minimum_required that newer CMake
# versions reject. Setting CMAKE_POLICY_VERSION_MINIMUM=3.5 here
# tells CMake to allow the old version anyway.
set(CMAKE_POLICY_VERSION_MINIMUM 3.5)