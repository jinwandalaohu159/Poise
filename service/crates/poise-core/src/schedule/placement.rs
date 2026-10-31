//! 让到副域（小核）的程序放在哪个核心域（见 `CoreSet`）：
//! - 同一个程序的所有进程放在同一个域：线程之间共享的数据留在同一块缓存里，不跨缓存搬运；
//! - 不同的程序按各域的占用比例分摊（域大小不同时按比例算），互相不挤；
//! - 分到某个域后就留在那里（挪动会丢掉缓存里的数据）。只有两种情况会挪：那个域要留给重点程序
//!   （见 `Footprint`）；那个域已经超负荷、而别的域还很空——每轮最多挪一个，逐步摊开，不来回挪；
//! - 一个程序的负载比一个域能承担的还大时不限制在一个域里，用全部可用的副域：不为了缓存把它挤在几个核心上。

use std::collections::{HashMap, HashSet};

use super::CoreLayout;
use crate::action::CoreSet;
use crate::model::ProcessKey;

#[derive(Debug, Default)]
pub struct GroupPlacer {
    /// 程序（按主进程）分到的域。
    assigned: HashMap<ProcessKey, u8>,
}

impl GroupPlacer {
    /// 负载超过一个域能力的这个比例时，不再限制在一个域里。
    const FITS_DOMAIN: f32 = 0.75;
    /// 分摊时每个程序至少按这么多负载算：闲着的程序也均匀分开，不全堆在同一个域。
    const MIN_WEIGHT: f32 = 1.0;
    /// 挪过去之后，目标域的占用不超过这个比例才挪（挪过去不能让它也超负荷）。
    const REBALANCE_TARGET: f32 = 0.75;

    /// `wants` 是要让到副域的程序及其负载（单核百分比），`reserved` 是留给重点程序的域（位掩码）。
    /// 返回每个程序的核心范围。
    pub fn place(
        &mut self,
        wants: &[(ProcessKey, f32)],
        layout: CoreLayout,
        reserved: u16,
    ) -> HashMap<ProcessKey, CoreSet> {
        let away: Vec<u8> = (1..layout.domains()).collect();
        let mut usable: Vec<u8> = away
            .iter()
            .copied()
            .filter(|d| reserved & (1 << d) == 0)
            .collect();
        // 副域全被重点程序占了：只能一起用。
        if usable.is_empty() {
            usable = away;
        }
        match usable.as_slice() {
            [] => {
                return wants
                    .iter()
                    .map(|(k, _)| (*k, CoreSet::Efficiency))
                    .collect();
            }
            &[only] => {
                for (k, _) in wants {
                    self.assigned.insert(*k, only);
                }
                return wants
                    .iter()
                    .map(|(k, _)| (*k, CoreSet::Domains(1 << only)))
                    .collect();
            }
            _ => {}
        }
        let capacity = |d: u8| layout.capacity(d);
        let all_usable = usable.iter().fold(0u16, |m, d| m | (1 << d));
        let mut load = vec![0.0_f32; usize::from(layout.domains())];
        let mut out = HashMap::with_capacity(wants.len());
        let mut sticky = Vec::new();
        let mut fresh = Vec::new();
        for &(root, pct) in wants {
            let largest = usable.iter().map(|&d| capacity(d)).fold(0.0, f32::max);
            if pct > largest * Self::FITS_DOMAIN {
                for &d in &usable {
                    load[usize::from(d)] += pct / usable.len() as f32;
                }
                out.insert(root, CoreSet::Domains(all_usable));
                continue;
            }
            match self.assigned.get(&root) {
                Some(d) if usable.contains(d) => {
                    load[usize::from(*d)] += pct.max(Self::MIN_WEIGHT);
                    sticky.push((root, pct));
                }
                _ => fresh.push((root, pct)),
            }
        }
        // 新来的程序从重到轻，每个放进放下之后占用比例最低的域。
        fresh.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.pid.cmp(&b.0.pid)));
        for (root, pct) in fresh {
            let weight = pct.max(Self::MIN_WEIGHT);
            let fill = |d: u8| (load[usize::from(d)] + weight) / capacity(d);
            let d = *usable
                .iter()
                .min_by(|a, b| fill(**a).total_cmp(&fill(**b)))
                .expect("至少两个可用的域");
            load[usize::from(d)] += weight;
            self.assigned.insert(root, d);
        }
        self.rebalance_one(&sticky, &usable, &mut load, capacity);
        for (root, _) in wants {
            if let Some(&d) = self.assigned.get(root) {
                out.entry(*root).or_insert(CoreSet::Domains(1 << d));
            }
        }
        out
    }

    /// 某个域超负荷、别的域还很空时，把那里一个程序挪过去（能挪走的里面最轻的那个）。
    fn rebalance_one(
        &mut self,
        sticky: &[(ProcessKey, f32)],
        usable: &[u8],
        load: &mut [f32],
        capacity: impl Fn(u8) -> f32,
    ) {
        let fill = |load: &[f32], d: u8| load[usize::from(d)] / capacity(d);
        let Some(&from) = usable
            .iter()
            .filter(|&&d| fill(load, d) > 1.0)
            .max_by(|a, b| fill(load, **a).total_cmp(&fill(load, **b)))
        else {
            return;
        };
        let mut movable: Vec<&(ProcessKey, f32)> = sticky
            .iter()
            .filter(|(k, _)| self.assigned.get(k) == Some(&from))
            .collect();
        movable.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.pid.cmp(&b.0.pid)));
        for &&(root, pct) in &movable {
            let weight = pct.max(Self::MIN_WEIGHT);
            let target = usable
                .iter()
                .copied()
                .filter(|&d| d != from)
                .filter(|&d| {
                    (load[usize::from(d)] + weight) / capacity(d) <= Self::REBALANCE_TARGET
                })
                .min_by(|a, b| fill(load, *a).total_cmp(&fill(load, *b)));
            if let Some(to) = target {
                load[usize::from(from)] -= weight;
                load[usize::from(to)] += weight;
                self.assigned.insert(root, to);
                return;
            }
        }
    }

    pub fn retain_alive(&mut self, alive: &HashSet<ProcessKey>) {
        self.assigned.retain(|k, _| alive.contains(k));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alder() -> CoreLayout {
        CoreLayout::new(&[12, 4, 4])
    }

    fn key(pid: u32) -> ProcessKey {
        ProcessKey { pid, created: 1 }
    }

    const G0: CoreSet = CoreSet::Domains(0b010);
    const G1: CoreSet = CoreSet::Domains(0b100);

    #[test]
    fn programs_are_spread_by_load_and_stay_in_their_domain() {
        let mut p = GroupPlacer::default();
        let wants = [(key(1), 200.0), (key(2), 150.0), (key(3), 40.0)];
        let placed = p.place(&wants, alder(), 0);
        assert_eq!(placed[&key(1)], G0);
        assert_eq!(placed[&key(2)], G1);
        assert_eq!(placed[&key(3)], G1);
        // 负载变了也不挪：留在原来的域。
        let wants = [(key(1), 10.0), (key(2), 250.0), (key(3), 40.0)];
        assert_eq!(p.place(&wants, alder(), 0), placed);
    }

    #[test]
    fn an_overloaded_domain_is_relieved_one_program_per_round() {
        let mut p = GroupPlacer::default();
        // 留出第一组时，全部放进第二组。
        let wants = [(key(1), 150.0), (key(2), 150.0), (key(3), 150.0)];
        let placed = p.place(&wants, alder(), 0b010);
        assert!(placed.values().all(|c| *c == G1));
        // 不再留出：第二组超负荷（450% > 400%），每轮挪走一个，直到不超负荷。
        let placed = p.place(&wants, alder(), 0);
        assert_eq!(placed.values().filter(|c| **c == G0).count(), 1);
        let placed = p.place(&wants, alder(), 0);
        assert_eq!(placed.values().filter(|c| **c == G0).count(), 1);
    }

    #[test]
    fn heavy_programs_span_every_usable_domain() {
        let mut p = GroupPlacer::default();
        // 一组 4 个小核（400%），超过 300% 不限制在一组里。
        let wants = [(key(1), 350.0)];
        assert_eq!(
            p.place(&wants, alder(), 0)[&key(1)],
            CoreSet::Domains(0b110)
        );
        // 第一组留给重点程序：只剩第二组可用。
        assert_eq!(p.place(&wants, alder(), 0b010)[&key(1)], G1);
    }

    #[test]
    fn domains_of_different_sizes_are_filled_in_proportion() {
        let mut p = GroupPlacer::default();
        // 主域之外：一个 8 线程的域、一个 2 线程的域（例如低功耗小核）。
        let layout = CoreLayout::new(&[8, 8, 2]);
        let wants: Vec<_> = (1..=10).map(|i| (key(i), 20.0)).collect();
        let placed = p.place(&wants, layout, 0);
        let small = placed
            .values()
            .filter(|c| **c == CoreSet::Domains(0b100))
            .count();
        assert_eq!(small, 2);
    }

    #[test]
    fn a_single_away_domain_needs_no_choice() {
        let mut p = GroupPlacer::default();
        let one = CoreLayout::new(&[12, 8]);
        assert_eq!(
            p.place(&[(key(1), 50.0)], one, 0)[&key(1)],
            CoreSet::Domains(0b10)
        );
        // 只有一个域的处理器：交给调用方（不限制核心）。
        let uniform = CoreLayout::uniform(16);
        assert_eq!(
            p.place(&[(key(1), 50.0)], uniform, 0)[&key(1)],
            CoreSet::Efficiency
        );
    }
}
