// Copyright 2022 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "common.h"

namespace netstack_syscall_test {

void AddSkippedTestsBacklogAcceptBacklogShared(TestMap& tests) {
  SkipTest(tests, "All/DualStackSocketTest.AddressOperations/*");
  SkipTest(tests, "SocketInetLoopbackTest.LoopbackAddressRangeConnect");
  SkipTest(tests, "BadSocketPairArgs.ValidateErrForBadCallsToSocketPair");

  SkipTest(tests, "All/SocketInetLoopbackTest.TCP/*");
  SkipTest(tests, "All/SocketInetLoopbackTest.TCPListenUnbound/*");
  SkipTest(tests, "All/SocketInetLoopbackTest.TCPListenShutdownListen/*");
  SkipTest(tests, "All/SocketInetLoopbackTest.TCPListenShutdown/*");
  SkipTest(tests, "All/SocketInetLoopbackTest.TCPListenClose/*");
  SkipTest(tests, "All/SocketInetLoopbackTest.TCPInfoState/*");
  SkipTest(tests, "All/SocketInetLoopbackTest.TCPListenCloseDuringConnect/*");
  SkipTest(tests, "All/SocketInetLoopbackTest.TCPListenShutdownDuringConnect/*");
  SkipTest(tests, "All/SocketInetLoopbackTest.TCPListenCloseConnectingRead/*");
  SkipTest(tests, "All/SocketInetLoopbackTest.TCPListenShutdownConnectingRead/*");
  SkipTest(tests, "All/SocketInetLoopbackTest.TCPNonBlockingConnectClose/*");
  SkipTest(tests, "All/SocketInetLoopbackTest.TCPResetAfterClose/*");
  SkipTest(tests, "All/SocketInetLoopbackTest.AcceptedInheritsTCPUserTimeout/*");
  SkipTest(tests, "All/SocketInetLoopbackTest.TCPAcceptAfterReset/*");
  SkipTest(tests, "All/SocketInetLoopbackTest.TCPDeferAccept/*");
  SkipTest(tests, "All/SocketInetLoopbackTest.TCPDeferAcceptTimeout/*");
  SkipTest(tests, "All/SocketInetLoopbackTest.TCPDeferAcceptTimeout/*");
  SkipTest(tests, "All/SocketInetLoopbackTest.TCPDeferAcceptTimeout/*");

  SkipTest(tests, "All/SocketInetReusePortTest.TcpPortReuseMultiThread/*");
  SkipTest(tests, "All/SocketInetReusePortTest.UdpPortReuseMultiThread/*");
  SkipTest(tests, "All/SocketInetReusePortTest.UdpPortReuseMultiThreadShort/*");

  SkipTest(tests,
           "AllFamilies/SocketMultiProtocolInetLoopbackTest.V4MappedLoopbackOnlyReservesV4/*");
  SkipTest(tests, "AllFamilies/SocketMultiProtocolInetLoopbackTest.V4MappedAnyOnlyReservesV4/*");
  SkipTest(tests,
           "AllFamilies/SocketMultiProtocolInetLoopbackTest.DualStackV6AnyReservesEverything/*");
  SkipTest(tests,
           "AllFamilies/"
           "SocketMultiProtocolInetLoopbackTest.DualStackV6AnyReuseAddrDoesNotReserveV4Any/*");
  SkipTest(tests,
           "AllFamilies/"
           "SocketMultiProtocolInetLoopbackTest.DualStackV6AnyReuseAddrListenReservesV4Any/*");
  SkipTest(tests,
           "AllFamilies/"
           "SocketMultiProtocolInetLoopbackTest.DualStackV6AnyWithListenReservesEverything/*");
  SkipTest(tests, "AllFamilies/SocketMultiProtocolInetLoopbackTest.V6OnlyV6AnyReservesV6/*");
  SkipTest(tests, "AllFamilies/SocketMultiProtocolInetLoopbackTest.V6EphemeralPortReserved/*");
  SkipTest(tests,
           "AllFamilies/SocketMultiProtocolInetLoopbackTest.V4MappedEphemeralPortReserved/*");
  SkipTest(tests, "AllFamilies/SocketMultiProtocolInetLoopbackTest.V4EphemeralPortReserved/*");
  SkipTest(
      tests,
      "AllFamilies/SocketMultiProtocolInetLoopbackTest.MultipleBindsAllowedNoListeningReuseAddr/*");
  SkipTest(tests, "AllFamilies/SocketMultiProtocolInetLoopbackTest.PortReuseTwoSockets/*");
  SkipTest(tests,
           "AllFamilies/SocketMultiProtocolInetLoopbackTest.NoReusePortFollowingReusePort/*");
}

void AddSkippedTestsTcpAcceptBacklog(TestMap& tests) {
  AddSkippedTestsBacklogAcceptBacklogShared(tests);
  SkipTest(tests, "All/SocketInetLoopbackTest.TCPBacklog/*");
  SkipTest(tests, "All/SocketInetLoopbackTest.TCPBacklogAcceptAll/*");
}

void AddSkippedTestsTcpBacklog(TestMap& tests) {
  AddSkippedTestsBacklogAcceptBacklogShared(tests);
  SkipTest(tests, "All/SocketInetLoopbackTest.TCPAcceptBacklogSizes/*");
}

void AddSkippedTestsLoopback(TestMap& tests) {
  SkipTest(tests, "All/SocketInetLoopbackTest.TCPAcceptBacklogSizes/*");
  SkipTest(tests, "All/SocketInetLoopbackTest.TCPBacklog/*");
}

void AddSkippedTestsLoopbackIsolated(TestMap& tests) {
  SkipTest(tests, "All/SocketInetLoopbackIsolatedTest.TCPFinWait2Test/*");
  SkipTest(tests, "All/SocketInetLoopbackIsolatedTest.TCPLinger2TimeoutAfterClose/*");
}

void AddSkippedTestsFinWaitLingerTimeoutShared(TestMap& tests) {
  SkipTest(tests, "All/SocketInetLoopbackIsolatedTest.TCPActiveCloseTimeWaitTest/*");
  SkipTest(tests, "All/SocketInetLoopbackIsolatedTest.TCPActiveCloseTimeWaitReuseTest/*");
  SkipTest(tests, "All/SocketInetLoopbackIsolatedTest.TCPPassiveCloseNoTimeWaitTest/*");
  SkipTest(tests, "All/SocketInetLoopbackIsolatedTest.TCPPassiveCloseNoTimeWaitReuseTest/*");
  SkipTest(tests,
           "AllFamilies/SocketMultiProtocolInetLoopbackIsolatedTest.BindToDeviceReusePort/*");
  SkipTest(
      tests,
      "AllFamilies/SocketMultiProtocolInetLoopbackIsolatedTest.V4EphemeralPortReservedReuseAddr/*");
  SkipTest(tests,
           "AllFamilies/"
           "SocketMultiProtocolInetLoopbackIsolatedTest.V4MappedEphemeralPortReservedReuseAddr/*");
  SkipTest(
      tests,
      "AllFamilies/SocketMultiProtocolInetLoopbackIsolatedTest.V6EphemeralPortReservedReuseAddr/*");
}

void AddSkippedTestsFinWait(TestMap& tests) {
  AddSkippedTestsFinWaitLingerTimeoutShared(tests);
  SkipTest(tests, "All/SocketInetLoopbackIsolatedTest.TCPLinger2TimeoutAfterClose/*");
}

void AddSkippedTestsLingerTimeout(TestMap& tests) {
  AddSkippedTestsFinWaitLingerTimeoutShared(tests);
  SkipTest(tests, "All/SocketInetLoopbackIsolatedTest.TCPFinWait2Test/*");
}

}  // namespace netstack_syscall_test
