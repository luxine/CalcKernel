// Dijkstra shortest paths over a dense adjacency matrix.
// weights[row * node_count + col] is the edge weight.
// A weight <= 0 means there is no directed edge.

struct DijkstraConfig {
  node_count: i32;
  source: i32;
  inf: i64;
}

fn matrix_index(row: i32, col: i32, node_count: i32) -> i32 {
  return row * node_count + col;
}

fn is_unvisited(visited: ptr<i32>, index: i32) -> bool {
  return visited[index] == 0;
}

fn should_relax(current_dist: i64, edge_weight: i64, target_dist: i64, inf: i64) -> bool {
  if current_dist >= inf {
    return false;
  }

  if edge_weight <= 0 {
    return false;
  }

  return current_dist + edge_weight < target_dist;
}

export fn dijkstra_matrix(
  configs: ptr<DijkstraConfig>,
  weights: ptr<i64>,
  dist_out: ptr<i64>,
  prev_out: ptr<i32>,
  visited: ptr<i32>
) -> i32 {
  let node_count: i32 = configs[0].node_count;
  let source: i32 = configs[0].source;
  let inf: i64 = configs[0].inf;
  let i: i32 = 0;

  while i < node_count {
    dist_out[i] = inf;
    prev_out[i] = -1;
    visited[i] = 0;
    i = i + 1;
  }

  dist_out[source] = 0;
  prev_out[source] = source;

  let settled_count: i32 = 0;

  while settled_count < node_count {
    let scan: i32 = 0;
    let best_node: i32 = -1;
    let best_dist: i64 = inf;

    while scan < node_count {
      if is_unvisited(visited, scan) && dist_out[scan] < best_dist {
        best_node = scan;
        best_dist = dist_out[scan];
      }
      scan = scan + 1;
    }

    if best_node < 0 {
      return settled_count;
    }

    visited[best_node] = 1;

    let neighbor: i32 = 0;

    while neighbor < node_count {
      let edge_index: i32 = matrix_index(best_node, neighbor, node_count);
      let edge_weight: i64 = weights[edge_index];

      if is_unvisited(visited, neighbor) && should_relax(best_dist, edge_weight, dist_out[neighbor], inf) {
        let candidate: i64 = best_dist + edge_weight;
        dist_out[neighbor] = candidate;
        prev_out[neighbor] = best_node;
      }

      neighbor = neighbor + 1;
    }

    settled_count = settled_count + 1;
  }

  return settled_count;
}
