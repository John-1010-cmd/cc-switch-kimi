import { useTranslation } from "react-i18next";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useModelStats } from "@/lib/query/usage";
import { fmtUsd } from "./format";
import type { UsageRangeSelection } from "@/types/usage";

interface ModelStatsTableProps {
  range: UsageRangeSelection;
  appType?: string;
  providerName?: string;
  model?: string;
  refreshIntervalMs: number;
}

const COLUMN_COUNT = 8;

export function ModelStatsTable({
  range,
  appType,
  providerName,
  model,
  refreshIntervalMs,
}: ModelStatsTableProps) {
  const { t } = useTranslation();
  const { data: stats, isLoading } = useModelStats(
    range,
    { appType, providerName, model },
    {
      refetchInterval: refreshIntervalMs > 0 ? refreshIntervalMs : false,
    },
  );

  if (isLoading) {
    return <div className="h-[400px] animate-pulse rounded bg-gray-100" />;
  }

  return (
    <div className="rounded-lg border border-border/50 bg-card/40 backdrop-blur-sm overflow-hidden">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{t("usage.model", "模型")}</TableHead>
            <TableHead className="text-right">
              {t("usage.requests", "请求数")}
            </TableHead>
            <TableHead className="text-right">
              {t("usage.inputTokens", "输入")}
            </TableHead>
            <TableHead className="text-right">
              {t("usage.outputTokens", "输出")}
            </TableHead>
            <TableHead className="text-right">
              {t("usage.cacheReadTokens", "缓存读取")}
            </TableHead>
            <TableHead className="text-right">
              {t("usage.cacheHitRate", "缓存命中率")}
            </TableHead>
            <TableHead className="text-right">
              {t("usage.totalCost", "总成本")}
            </TableHead>
            <TableHead className="text-right">
              {t("usage.avgCost", "平均成本")}
            </TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {stats?.length === 0 ? (
            <TableRow>
              <TableCell
                colSpan={COLUMN_COUNT}
                className="text-center text-muted-foreground"
              >
                {t("usage.noData", "暂无数据")}
              </TableCell>
            </TableRow>
          ) : (
            stats?.map((stat) => (
              <TableRow key={stat.model}>
                <TableCell className="font-mono text-sm">
                  {stat.model}
                </TableCell>
                <TableCell className="text-right">
                  {stat.requestCount.toLocaleString()}
                </TableCell>
                <TableCell className="text-right">
                  {stat.totalInputTokens.toLocaleString()}
                </TableCell>
                <TableCell className="text-right">
                  {stat.totalOutputTokens.toLocaleString()}
                </TableCell>
                <TableCell className="text-right">
                  {stat.totalCacheReadTokens.toLocaleString()}
                </TableCell>
                <CacheHitRateCell
                  rate={stat.cacheHitRate}
                  cacheable={
                    stat.totalInputTokens +
                    stat.totalCacheCreationTokens +
                    stat.totalCacheReadTokens
                  }
                />
                <TableCell className="text-right">
                  {fmtUsd(stat.totalCost, 4)}
                </TableCell>
                <TableCell className="text-right">
                  {fmtUsd(stat.avgCostPerRequest, 6)}
                </TableCell>
              </TableRow>
            ))
          )}
        </TableBody>
      </Table>
    </div>
  );
}

/** 缓存命中率列：无可缓存输入时显示 "—"（0% 具有误导性） */
function CacheHitRateCell({
  rate,
  cacheable,
}: {
  rate: number;
  cacheable: number;
}) {
  if (cacheable <= 0) {
    return (
      <TableCell className="text-right text-muted-foreground/70">—</TableCell>
    );
  }
  return (
    <TableCell className="text-right">{(rate * 100).toFixed(1)}%</TableCell>
  );
}
