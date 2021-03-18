// automatically generated by stateify.

package ipv4

import (
	"gvisor.dev/gvisor/pkg/state"
)

func (i *icmpv4DestinationUnreachableSockError) StateTypeName() string {
	return "pkg/tcpip/network/ipv4.icmpv4DestinationUnreachableSockError"
}

func (i *icmpv4DestinationUnreachableSockError) StateFields() []string {
	return []string{}
}

func (i *icmpv4DestinationUnreachableSockError) beforeSave() {}

func (i *icmpv4DestinationUnreachableSockError) StateSave(stateSinkObject state.Sink) {
	i.beforeSave()
}

func (i *icmpv4DestinationUnreachableSockError) afterLoad() {}

func (i *icmpv4DestinationUnreachableSockError) StateLoad(stateSourceObject state.Source) {
}

func (i *icmpv4DestinationHostUnreachableSockError) StateTypeName() string {
	return "pkg/tcpip/network/ipv4.icmpv4DestinationHostUnreachableSockError"
}

func (i *icmpv4DestinationHostUnreachableSockError) StateFields() []string {
	return []string{
		"icmpv4DestinationUnreachableSockError",
	}
}

func (i *icmpv4DestinationHostUnreachableSockError) beforeSave() {}

func (i *icmpv4DestinationHostUnreachableSockError) StateSave(stateSinkObject state.Sink) {
	i.beforeSave()
	stateSinkObject.Save(0, &i.icmpv4DestinationUnreachableSockError)
}

func (i *icmpv4DestinationHostUnreachableSockError) afterLoad() {}

func (i *icmpv4DestinationHostUnreachableSockError) StateLoad(stateSourceObject state.Source) {
	stateSourceObject.Load(0, &i.icmpv4DestinationUnreachableSockError)
}

func (i *icmpv4DestinationPortUnreachableSockError) StateTypeName() string {
	return "pkg/tcpip/network/ipv4.icmpv4DestinationPortUnreachableSockError"
}

func (i *icmpv4DestinationPortUnreachableSockError) StateFields() []string {
	return []string{
		"icmpv4DestinationUnreachableSockError",
	}
}

func (i *icmpv4DestinationPortUnreachableSockError) beforeSave() {}

func (i *icmpv4DestinationPortUnreachableSockError) StateSave(stateSinkObject state.Sink) {
	i.beforeSave()
	stateSinkObject.Save(0, &i.icmpv4DestinationUnreachableSockError)
}

func (i *icmpv4DestinationPortUnreachableSockError) afterLoad() {}

func (i *icmpv4DestinationPortUnreachableSockError) StateLoad(stateSourceObject state.Source) {
	stateSourceObject.Load(0, &i.icmpv4DestinationUnreachableSockError)
}

func (e *icmpv4FragmentationNeededSockError) StateTypeName() string {
	return "pkg/tcpip/network/ipv4.icmpv4FragmentationNeededSockError"
}

func (e *icmpv4FragmentationNeededSockError) StateFields() []string {
	return []string{
		"icmpv4DestinationUnreachableSockError",
		"mtu",
	}
}

func (e *icmpv4FragmentationNeededSockError) beforeSave() {}

func (e *icmpv4FragmentationNeededSockError) StateSave(stateSinkObject state.Sink) {
	e.beforeSave()
	stateSinkObject.Save(0, &e.icmpv4DestinationUnreachableSockError)
	stateSinkObject.Save(1, &e.mtu)
}

func (e *icmpv4FragmentationNeededSockError) afterLoad() {}

func (e *icmpv4FragmentationNeededSockError) StateLoad(stateSourceObject state.Source) {
	stateSourceObject.Load(0, &e.icmpv4DestinationUnreachableSockError)
	stateSourceObject.Load(1, &e.mtu)
}

func init() {
	state.Register((*icmpv4DestinationUnreachableSockError)(nil))
	state.Register((*icmpv4DestinationHostUnreachableSockError)(nil))
	state.Register((*icmpv4DestinationPortUnreachableSockError)(nil))
	state.Register((*icmpv4FragmentationNeededSockError)(nil))
}
