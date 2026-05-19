import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { motion } from "framer-motion";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Card, CardContent } from "@/components/ui/card";
import { useModelDetailStats } from "@/lib/query/usage";
import {
  formatTokensShort,
  fmtUsd,
  getResolvedLang,
  parseFiniteNumber,
} from "./format";
import type { ModelDetailStats, UsageRangeSelection } from "@/types/usage";
import { Loader2 } from "lucide-react";

interface ModelRankingPageProps {
  range: UsageRangeSelection;
  appType?: string;
  refreshIntervalMs: number;
}

export function ModelRankingPage({
  range,
  appType,
  refreshIntervalMs,
}: ModelRankingPageProps) {
  const { t, i18n } = useTranslation();
  const lang = getResolvedLang(i18n);

  const { data: stats, isLoading } = useModelDetailStats(range, appType, {
    refetchInterval: refreshIntervalMs > 0 ? refreshIntervalMs : false,
  });

  const summary = useMemo(() => {
    if (!stats || stats.length === 0) return null;
    let totalTokens = 0;
    let inputTokens = 0;
    let outputTokens = 0;
    let cacheCreationTokens = 0;
    let cacheReadTokens = 0;
    let totalRequests = 0;
    let totalCost = 0;
    for (const s of stats) {
      totalTokens += s.totalTokens;
      inputTokens += s.inputTokens;
      outputTokens += s.outputTokens;
      cacheCreationTokens += s.cacheCreationTokens;
      cacheReadTokens += s.cacheReadTokens;
      totalRequests += s.requestCount;
      totalCost += parseFiniteNumber(s.totalCost) ?? 0;
    }
    const cacheableInput = inputTokens + cacheCreationTokens + cacheReadTokens;
    const cacheHitRate =
      cacheableInput > 0 ? cacheReadTokens / cacheableInput : 0;
    return {
      totalTokens,
      inputTokens,
      outputTokens,
      cacheCreationTokens,
      cacheReadTokens,
      totalRequests,
      totalCost,
      cacheHitRate,
    };
  }, [stats]);

  if (isLoading) {
    return (
      <div className="flex items-center justify-center min-h-[400px]">
        <Loader2 className="h-6 w-6 animate-spin text-muted-foreground/50" />
      </div>
    );
  }

  if (!stats || stats.length === 0) {
    return (
      <div className="text-center text-muted-foreground py-12">
        {t("usage.noData", "暂无数据")}
      </div>
    );
  }

  return (
    <motion.div
      initial={{ opacity: 0, y: 10 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.4 }}
      className="space-y-8"
    >
      {/* Hero summary cards */}
      {summary && (
        <HeroSummary summary={summary} lang={lang} />
      )}

      {/* Model ranking table */}
      <div className="space-y-3">
        <h3 className="text-lg font-semibold">
          {t("usage.modelRanking", "模型排名")}
        </h3>
        <ModelRankingTable
          stats={stats}
          totalTokens={summary?.totalTokens ?? 0}
          lang={lang}
        />
      </div>

      {/* Model detail table */}
      <div className="space-y-3">
        <h3 className="text-lg font-semibold">
          {t("usage.modelDetail", "模型明细")}
        </h3>
        <ModelDetailTable stats={stats} lang={lang} />
      </div>
    </motion.div>
  );
}

interface SummaryData {
  totalTokens: number;
  inputTokens: number;
  outputTokens: number;
  cacheCreationTokens: number;
  cacheReadTokens: number;
  totalRequests: number;
  totalCost: number;
  cacheHitRate: number;
}

interface HeroSummaryProps {
  summary: SummaryData;
  lang: string;
}

function HeroSummary({ summary, lang }: HeroSummaryProps) {
  const { t } = useTranslation();
  const hitPercent = Math.max(0, Math.min(100, summary.cacheHitRate * 100));
  const hitPercentLabel = hitPercent.toFixed(hitPercent >= 99.95 ? 0 : 1);

  const cards = [
    {
      label: t("usage.realTotal", "总 Token"),
      value: formatTokensShort(summary.totalTokens, lang),
      sub: `${formatTokensShort(summary.inputTokens, lang)} + ${formatTokensShort(summary.outputTokens, lang)}`,
    },
    {
      label: t("usage.freshInput", "输入 Token"),
      value: formatTokensShort(summary.inputTokens, lang),
      sub: t("usage.cacheWrite", "缓存写入") + " " + formatTokensShort(summary.cacheCreationTokens, lang),
    },
    {
      label: t("usage.output", "输出 Token"),
      value: formatTokensShort(summary.outputTokens, lang),
      sub: t("usage.cacheRead", "缓存读取") + " " + formatTokensShort(summary.cacheReadTokens, lang),
    },
    {
      label: t("usage.totalRequests", "请求数"),
      value: summary.totalRequests.toLocaleString(),
    },
    {
      label: t("usage.cacheHitRate", "缓存命中率"),
      value: `${hitPercentLabel}%`,
    },
    {
      label: t("usage.estimatedCost", "预估费用"),
      value: fmtUsd(summary.totalCost, 4),
    },
  ];

  return (
    <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-6 gap-3">
      {cards.map((card) => (
        <Card
          key={card.label}
          className="border border-border/50 bg-card/40 backdrop-blur-sm"
        >
          <CardContent className="p-4">
            <div className="text-xs text-muted-foreground mb-1">
              {card.label}
            </div>
            <div className="text-xl font-bold tabular-nums">{card.value}</div>
            {card.sub && (
              <div className="text-xs text-muted-foreground mt-1">
                {card.sub}
              </div>
            )}
          </CardContent>
        </Card>
      ))}
    </div>
  );
}

interface ModelRankingTableProps {
  stats: ModelDetailStats[];
  totalTokens: number;
  lang: string;
}

function ModelRankingTable({ stats, totalTokens, lang }: ModelRankingTableProps) {
  const { t } = useTranslation();

  return (
    <div className="rounded-lg border border-border/50 bg-card/40 backdrop-blur-sm overflow-hidden">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{t("usage.model", "模型")}</TableHead>
            <TableHead className="w-[120px]">
              {t("usage.usageRatio", "用量占比")}
            </TableHead>
            <TableHead className="text-right">
              {t("usage.realTotal", "总 Token")}
            </TableHead>
            <TableHead className="text-right">
              {t("usage.input", "输入")}
            </TableHead>
            <TableHead className="text-right">
              {t("usage.output", "输出")}
            </TableHead>
            <TableHead className="text-right">
              {t("usage.totalRequests", "请求数")}
            </TableHead>
            <TableHead className="text-right">
              {t("usage.estimatedCost", "预估费用")}
            </TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {stats.map((stat) => {
            const pct =
              totalTokens > 0 ? (stat.totalTokens / totalTokens) * 100 : 0;
            return (
              <TableRow key={stat.model}>
                <TableCell className="font-mono text-sm">
                  {stat.model}
                </TableCell>
                <TableCell>
                  <div className="flex items-center gap-2">
                    <div className="flex-1 h-2 rounded-full bg-muted/50 overflow-hidden">
                      <motion.div
                        className="h-full bg-primary/70 rounded-full"
                        initial={{ width: 0 }}
                        animate={{ width: `${pct}%` }}
                        transition={{ duration: 0.6, ease: "easeOut" }}
                      />
                    </div>
                    <span className="text-xs text-muted-foreground tabular-nums w-[40px] text-right">
                      {pct.toFixed(1)}%
                    </span>
                  </div>
                </TableCell>
                <TableCell className="text-right tabular-nums">
                  {formatTokensShort(stat.totalTokens, lang)}
                </TableCell>
                <TableCell className="text-right tabular-nums">
                  {formatTokensShort(stat.inputTokens, lang)}
                </TableCell>
                <TableCell className="text-right tabular-nums">
                  {formatTokensShort(stat.outputTokens, lang)}
                </TableCell>
                <TableCell className="text-right tabular-nums">
                  {stat.requestCount.toLocaleString()}
                </TableCell>
                <TableCell className="text-right tabular-nums">
                  {fmtUsd(stat.totalCost, 4)}
                </TableCell>
              </TableRow>
            );
          })}
        </TableBody>
      </Table>
    </div>
  );
}

interface ModelDetailTableProps {
  stats: ModelDetailStats[];
  lang: string;
}

function ModelDetailTable({ stats, lang }: ModelDetailTableProps) {
  const { t } = useTranslation();

  return (
    <div className="rounded-lg border border-border/50 bg-card/40 backdrop-blur-sm overflow-hidden">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{t("usage.model", "模型")}</TableHead>
            <TableHead className="text-right">
              {t("usage.inputTokens", "输入 Token")}
            </TableHead>
            <TableHead className="text-right">
              {t("usage.outputTokens", "输出 Token")}
            </TableHead>
            <TableHead className="text-right">
              {t("usage.cacheWrite", "缓存写入")}
            </TableHead>
            <TableHead className="text-right">
              {t("usage.cacheRead", "缓存读取")}
            </TableHead>
            <TableHead className="text-right">
              {t("usage.totalRequests", "请求数")}
            </TableHead>
            <TableHead className="text-right">
              {t("usage.cacheHitRate", "缓存命中率")}
            </TableHead>
            <TableHead className="text-right">
              {t("usage.cost", "费用")}
            </TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {stats.map((stat) => {
            const hitRate = stat.cacheHitRate * 100;
            const hitRateLabel = hitRate.toFixed(hitRate >= 99.95 ? 0 : 1);
            const hasCache =
              stat.cacheCreationTokens > 0 || stat.cacheReadTokens > 0;
            return (
              <TableRow key={stat.model}>
                <TableCell className="font-mono text-sm">
                  {stat.model}
                </TableCell>
                <TableCell className="text-right tabular-nums">
                  {formatTokensShort(stat.inputTokens, lang)}
                </TableCell>
                <TableCell className="text-right tabular-nums">
                  {formatTokensShort(stat.outputTokens, lang)}
                </TableCell>
                <TableCell className="text-right tabular-nums">
                  {formatTokensShort(stat.cacheCreationTokens, lang)}
                </TableCell>
                <TableCell className="text-right tabular-nums">
                  {formatTokensShort(stat.cacheReadTokens, lang)}
                </TableCell>
                <TableCell className="text-right tabular-nums">
                  {stat.requestCount.toLocaleString()}
                </TableCell>
                <TableCell className="text-right tabular-nums">
                  {hasCache ? `${hitRateLabel}%` : "-"}
                </TableCell>
                <TableCell className="text-right tabular-nums">
                  {fmtUsd(stat.totalCost, 4)}
                </TableCell>
              </TableRow>
            );
          })}
        </TableBody>
      </Table>
    </div>
  );
}