// Copyright 2020 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "src/ui/scenic/lib/flatland/global_topology_data.h"

#include <lib/syslog/cpp/macros.h>

#include "src/ui/scenic/lib/utils/helpers.h"
#include "src/ui/scenic/lib/utils/logging.h"

namespace {

struct pair_hash {
  size_t operator()(const std::pair<flatland::TransformHandle, uint64_t>& p) const noexcept {
    return std::hash<flatland::TransformHandle>{}(p.first) ^ std::hash<uint64_t>{}(p.second);
  }
};

std::optional<zx_koid_t> GetViewRefKoid(
    const flatland::TransformHandle& handle,
    const std::unordered_map<flatland::TransformHandle,
                             std::shared_ptr<const fuchsia::ui::views::ViewRef>>& view_ref_map) {
  const auto kv = view_ref_map.find(handle);
  if (kv == view_ref_map.end()) {
    return std::nullopt;
  }

  return utils::ExtractKoid(*kv->second);
}

}  // namespace

namespace flatland {
using fuc_ViewportProperties = fuchsia::ui::composition::ViewportProperties;

// static
GlobalTopologyData GlobalTopologyData::ComputeGlobalTopologyData(
    const UberStruct::InstanceMap& uber_structs, const LinkTopologyMap& links,
    TransformHandle::InstanceId link_instance_id, TransformHandle root) {
  // There should never be an UberStruct for the |link_instance_id|.
  FX_DCHECK(uber_structs.find(link_instance_id) == uber_structs.end());

#ifdef USE_FLATLAND_VERBOSE_LOGGING
  {
    std::ostringstream str;
    str << "ComputeGlobalTopologyData(): Dumping UberStructs:\n";
    for (auto& kv : uber_structs) {
      str << *kv.second << "...................\n";
    }
    FLATLAND_VERBOSE_LOG << str.str();
  }
#endif

  // This is a stack of vector "iterators". We store the raw index, instead of an iterator, so that
  // we can do index comparisons.
  std::vector<std::pair<const TransformGraph::TopologyVector&, /*local_index=*/size_t>>
      vector_stack;
  // This is a stack of global parent indices and the number of children left to process for that
  // parent.
  struct ParentChildIterator {
    size_t parent_index = 0;
    size_t children_left = 0;
  };
  std::vector<ParentChildIterator> parent_counts;

  TopologyVector topology_vector;
  ChildCountVector child_counts;
  ParentIndexVector parent_indices;
  std::unordered_set<TransformHandle> live_transforms;
  std::unordered_map<TransformHandle, std::shared_ptr<const fuchsia::ui::views::ViewRef>> view_refs;
  std::unordered_map<TransformHandle, std::string> debug_names;
  std::unordered_map<TransformHandle, fuc_ViewportProperties> viewport_properties;

  // If we don't have the root in the map, the topology will be empty.
  const auto root_uber_struct_kv = uber_structs.find(root.GetInstanceId());
  if (root_uber_struct_kv != uber_structs.cend()) {
    vector_stack.emplace_back(root_uber_struct_kv->second->local_topology, 0);
  }

  while (!vector_stack.empty()) {
    auto& [vector, iterator_index] = vector_stack.back();

    // If we are finished with a vector, pop back to the previous vector.
    if (iterator_index >= vector.size()) {
      FX_DCHECK(iterator_index == vector.size());
      vector_stack.pop_back();
      continue;
    }

    const auto& current_entry = vector[iterator_index];
    FLATLAND_VERBOSE_LOG << "GlobalTopologyData processing current_entry=" << current_entry.handle
                         << "  child-count: " << current_entry.child_count;
    ++iterator_index;

    // Mark that a child has been processed for the latest parent.
    if (!parent_counts.empty()) {
      FLATLAND_VERBOSE_LOG << "GlobalTopologyData       parent_counts size: "
                           << parent_counts.size()
                           << "  parent: " << topology_vector[parent_counts.back().parent_index]
                           << "  remaining-children: " << parent_counts.back().children_left;

      FX_DCHECK(parent_counts.back().children_left > 0);
      --parent_counts.back().children_left;
    } else {
      // Only expect to see this at the root of the topology, I think.  Is this worth putting a
      // permanent check in?
      FLATLAND_VERBOSE_LOG << "GlobalTopologyData       no parent";
    }

    // If we are processing a link transform, find the other end of the link (if it exists).
    if (current_entry.handle.GetInstanceId() == link_instance_id) {
      // Decrement the parent's child count until the link is successfully resolved. An unresolved
      // link effectively means the parent had one fewer child.
      FX_DCHECK(!parent_counts.empty());
      auto& parent_child_count = child_counts[parent_counts.back().parent_index];
      --parent_child_count;

      // If the link doesn't exist, skip the link handle.
      const auto link_kv = links.find(current_entry.handle);
      if (link_kv == links.end()) {
        FLATLAND_VERBOSE_LOG << "GlobalTopologyData link doesn't exist for handle "
                             << current_entry.handle << ", skipping ";

        if (parent_counts.back().children_left == 0) {
          parent_counts.pop_back();
        }

        continue;
      }

      // If the link exists but doesn't have an UberStruct, skip the link handle.
      const auto uber_struct_kv = uber_structs.find(link_kv->second.GetInstanceId());
      if (uber_struct_kv == uber_structs.end()) {
        FLATLAND_VERBOSE_LOG << "GlobalTopologyData link doesn't exist for instance_id "
                             << link_kv->second.GetInstanceId() << ", skipping";

        if (parent_counts.back().children_left == 0) {
          parent_counts.pop_back();
        }

        continue;
      }

      // If the link exists and has an UberStruct but does not begin with the specified handle, skip
      // the new topology. This can occur if a new UberStruct has not been registered for the
      // corresponding instance ID but the link to it has resolved.
      const auto& new_vector = uber_struct_kv->second->local_topology;
      FX_DCHECK(!new_vector.empty()) << "Valid UberStructs cannot have empty local_topology";
      if (new_vector[0].handle != link_kv->second) {
        FLATLAND_VERBOSE_LOG << "GlobalTopologyData link mismatch with "
                                "existing UberStruct ("
                             << new_vector[0].handle << " vs. " << link_kv->second << "), skipping";

        if (parent_counts.back().children_left == 0) {
          parent_counts.pop_back();
        }

        continue;
      }

      // Thanks to one-view-per-session semantics, we should never cycle through the
      // topological vectors, so we don't need to handle cycles. We DCHECK here just to be sure.
      FX_DCHECK(std::find_if(vector_stack.cbegin(), vector_stack.cend(),
                             [&](std::pair<const TransformGraph::TopologyVector&, uint64_t> entry) {
                               return entry.first == new_vector;
                             }) == vector_stack.cend());

      // At this point, the link is resolved. This means the link did actually result in the parent
      // having an additional child, but that child needs to be processed, so the stack of remaining
      // children to process for each parent needs to be increment as well.
      ++parent_child_count;
      ++parent_counts.back().children_left;

      vector_stack.emplace_back(new_vector, 0);
      continue;
    }

    // Push the current transform and update the "iterator".
    const size_t new_parent_index = topology_vector.size();
    topology_vector.push_back(current_entry.handle);
    child_counts.push_back(current_entry.child_count);
    parent_indices.push_back(parent_counts.empty() ? 0 : parent_counts.back().parent_index);
    live_transforms.insert(current_entry.handle);

    // For the root of each local topology (i.e. the View), save the ViewRef if it has one.
    // Non-View roots might not have one, e.g. the display.
    if (current_entry == vector[0] &&
        uber_structs.at(current_entry.handle.GetInstanceId())->view_ref != nullptr) {
      view_refs.emplace(current_entry.handle,
                        uber_structs.at(current_entry.handle.GetInstanceId())->view_ref);
    }

    // For the root of each local topology (i.e. the View), save the debug name if it is not empty.
    if (current_entry == vector[0] &&
        !uber_structs.at(current_entry.handle.GetInstanceId())->debug_name.empty()) {
      debug_names.emplace(current_entry.handle,
                          uber_structs.at(current_entry.handle.GetInstanceId())->debug_name);
    }

    // For each node in the local topology, save the ViewportProperties of its child instances.
    for (auto& [child_handle, child_properties] :
         uber_structs.at(current_entry.handle.GetInstanceId())->link_properties) {
      fuc_ViewportProperties properties;
      fidl::Clone(child_properties, &properties);
      viewport_properties.try_emplace(child_handle, std::move(properties));
    }

    // If this entry was the last child for the previous parent, pop that off the stack.
    if (!parent_counts.empty() && parent_counts.back().children_left == 0) {
      parent_counts.pop_back();
    }

    // If this entry has children, push it onto the parent stack.
    if (current_entry.child_count != 0) {
      parent_counts.push_back({new_parent_index, current_entry.child_count});
    }
  }

// Validates that every child of every parent was processed. If the last handle processed was an
// unresolved link handle, its parent will be the only thing left on the stack with 0 children to
// avoid extra unnecessary cleanup logic.
#ifndef NDEBUG
  if (parent_counts.size() > 1 ||
      (parent_counts.size() == 1 && parent_counts.back().children_left != 0)) {
    std::ostringstream str;
    str << "Error while generating GlobalTopologyData (failed parent_counts validation)\n"
        << "Dumping parent_counts vector:\n";
    for (size_t i = 0; i < parent_counts.size(); ++i) {
      str << "i: " << i << "  index: " << parent_counts[i].parent_index
          << "  parent: " << topology_vector[parent_counts[i].parent_index]
          << "  child-count: " << parent_counts[i].children_left << std::endl;
    }
    FX_LOGS(FATAL) << str.str();
  }
  FX_CHECK(parent_counts.empty() ||
           (parent_counts.size() == 1 && parent_counts.back().children_left == 0));
#endif

  return {.topology_vector = std::move(topology_vector),
          .child_counts = std::move(child_counts),
          .parent_indices = std::move(parent_indices),
          .live_handles = std::move(live_transforms),
          .view_refs = std::move(view_refs),
          .debug_names = std::move(debug_names),
          .viewport_properties = std::move(viewport_properties)};
}

view_tree::SubtreeSnapshot GlobalTopologyData::GenerateViewTreeSnapshot(
    float display_width, float display_height, const GlobalTopologyData& data,
    const std::unordered_set<zx_koid_t>& view_ref_koids,
    const std::unordered_map<TransformHandle, TransformHandle>& child_view_watcher_mapping) {
  // Find the first node with a ViewRef set. This is the root of the ViewTree.
  size_t root_index = 0;
  while (root_index < data.topology_vector.size() &&
         data.view_refs.count(data.topology_vector[root_index]) == 0) {
    ++root_index;
  }

  // Didn't find one -> empty ViewTree.
  if (root_index == data.topology_vector.size()) {
    return {};
  }

  view_tree::SubtreeSnapshot snapshot{// We do not currently support other compositors as subtrees.
                                      .tree_boundaries = {}};
  auto& [root, view_tree, unconnected_views, hit_tester, tree_boundaries] = snapshot;

  // TODO(fxbug.dev/82677): Get real bounding boxes instead of using the full display size for each
  // one.
  const auto full_screen_bounding_box = view_tree::BoundingBox{
      .min = {0, 0},
      .max = {display_width, display_height},
  };

  // Add all Views to |view_tree|.
  root = GetViewRefKoid(data.topology_vector[root_index], data.view_refs).value();
  for (size_t i = root_index; i < data.topology_vector.size(); ++i) {
    const auto& transform_handle = data.topology_vector.at(i);
    if (data.view_refs.count(transform_handle) == 0) {
      // Transforms without ViewRefs are not Views and can be skipped.
      continue;
    }

    const auto& view_ref = data.view_refs.at(transform_handle);
    const zx_koid_t view_ref_koid = utils::ExtractKoid(*view_ref);

    std::string debug_name;
    if (data.debug_names.count(transform_handle) != 0)
      debug_name = data.debug_names.at(transform_handle);

    // Get the viewport_properties of a handle through its parent_viewport_watcher_handle.
    fuc_ViewportProperties properties;
    if (child_view_watcher_mapping.count(transform_handle) != 0) {
      auto& parent_viewport_watcher_handle = child_view_watcher_mapping.at(transform_handle);
      if (data.viewport_properties.count(parent_viewport_watcher_handle) != 0) {
        fidl::Clone(data.viewport_properties.at(parent_viewport_watcher_handle), &properties);
      }
    }

    // Find the parent by looking upwards until a View is found. The root has no parent.
    // TODO(fxbug.dev/84196): Disallow anonymous views from having parents?
    zx_koid_t parent_koid = ZX_KOID_INVALID;
    if (view_ref_koid != root) {
      size_t parent_index = data.parent_indices[i];
      while (data.view_refs.count(data.topology_vector[parent_index]) == 0) {
        parent_index = data.parent_indices[parent_index];
      }

      parent_koid = GetViewRefKoid(data.topology_vector[parent_index], data.view_refs).value();
    }

    // TODO(fxbug.dev/82678): Add local_from_world_transform to the ViewNode.
    view_tree.emplace(view_ref_koid,
                      view_tree::ViewNode{.parent = parent_koid,
                                          .bounding_box = full_screen_bounding_box,
                                          .view_ref = view_ref,
                                          .debug_name = debug_name,
                                          .viewport_properties = std::move(properties)});
  }

  // Fill in the children by deriving it from the parents of each node.
  for (const auto& [koid, view_node] : view_tree) {
    if (view_node.parent != ZX_KOID_INVALID) {
      view_tree.at(view_node.parent).children.emplace(koid);
    }
  }

  // TODO(fxbug.dev/72075): The hit tester currently directly returns the last leaf View instead of
  // doing a full hit test. This is a stopgap solution until we've designed the full hit testing API
  // for Flatland.
  zx_koid_t leaf_node_koid = ZX_KOID_INVALID;
  for (auto it = data.topology_vector.rbegin(); it != data.topology_vector.rend(); it++) {
    const std::optional<zx_koid_t> koid = GetViewRefKoid(*it, data.view_refs);
    if (koid.has_value()) {
      leaf_node_koid = koid.value();
      break;
    }
  }
  FX_DCHECK(leaf_node_koid != ZX_KOID_INVALID);
  // Note: The ViewTree represents a snapshot of the scene at a specific time. Because of this it's
  // important that it contains no references to live data. This means the hit testing closure must
  // contain only plain values or data with value semantics like shared_ptr<const>, to ensure that
  // it's safe to call from any thread.
  hit_tester = [leaf_node_koid](zx_koid_t start_node, glm::vec2 world_point,
                                bool is_semantic_hit_test) {
    return view_tree::SubtreeHitTestResult{.hits = {leaf_node_koid}};
  };

  // Add unconnected views to the snapshot.
  for (const auto view_ref_koid : view_ref_koids) {
    if (view_tree.count(view_ref_koid) == 0) {
      unconnected_views.insert(view_ref_koid);
    }
  }

  return snapshot;
}

}  // namespace flatland
