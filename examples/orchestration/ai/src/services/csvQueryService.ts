interface SourceDocument {
  id: string;
  title: string;
  metadata: {
    type: "text" | "pdf" | "web" | "csv";
    [key: string]: any;
  };
}

interface CSVData {
  sourceId: string;
  title: string;
  headers: string[];
  rows: Record<string, string>[];
}

interface CSVQueryResult {
  sourceId: string;
  title: string;
  matchingRows: Record<string, string>[];
  matchCount: number;
  searchTerms: string[];
}

export class CSVQueryService {
  private csvSources: Map<string, CSVData> = new Map();

  /**
   * Add a CSV source to the service
   */
  async addCSVSource(
    source: SourceDocument,
    csvContent: string,
  ): Promise<void> {
    try {
      console.log(`CSV Service: Adding CSV source ${source.id}`);

      const parsedData = this.parseCSV(csvContent);

      const csvData: CSVData = {
        sourceId: source.id,
        title: source.title,
        headers: parsedData.headers,
        rows: parsedData.rows,
      };

      this.csvSources.set(source.id, csvData);
      console.log(
        `CSV Service: Successfully added ${parsedData.rows.length} rows for source ${source.id}`,
      );
    } catch (error) {
      console.error(`CSV Service: Failed to add source ${source.id}:`, error);
      throw error;
    }
  }

  /**
   * Remove a CSV source
   */
  removeCSVSource(sourceId: string): void {
    if (this.csvSources.has(sourceId)) {
      this.csvSources.delete(sourceId);
      console.log(`CSV Service: Removed source ${sourceId}`);
    }
  }

  /**
   * Parse CSV content into structured data
   */
  private parseCSV(csvContent: string): {
    headers: string[];
    rows: Record<string, string>[];
  } {
    const lines = csvContent.trim().split("\n");
    if (lines.length === 0) {
      return { headers: [], rows: [] };
    }

    // Simple CSV parsing (you might want to use a proper CSV library like papaparse)
    const headers = this.parseCSVLine(lines[0]);
    const rows: Record<string, string>[] = [];

    for (let i = 1; i < lines.length; i++) {
      const values = this.parseCSVLine(lines[i]);
      if (values.length > 0) {
        const row: Record<string, string> = {};
        headers.forEach((header, index) => {
          row[header] = values[index] || "";
        });
        rows.push(row);
      }
    }

    return { headers, rows };
  }

  /**
   * Parse a single CSV line (handles basic quoting)
   */
  private parseCSVLine(line: string): string[] {
    const result: string[] = [];
    let current = "";
    let inQuotes = false;

    for (let i = 0; i < line.length; i++) {
      const char = line[i];

      if (char === '"') {
        inQuotes = !inQuotes;
      } else if (char === "," && !inQuotes) {
        result.push(current.trim());
        current = "";
      } else {
        current += char;
      }
    }

    result.push(current.trim());
    return result;
  }

  /**
   * Smart query that searches across CSV data
   */
  async smartQuery(
    query: string,
    maxResults: number = 3,
  ): Promise<CSVQueryResult[]> {
    const searchTerms = this.extractSearchTerms(query);
    const results: CSVQueryResult[] = [];

    for (const [sourceId, csvData] of this.csvSources) {
      const matchingRows = this.findMatchingRows(csvData, searchTerms);

      if (matchingRows.length > 0) {
        results.push({
          sourceId,
          title: csvData.title,
          matchingRows: matchingRows.slice(0, maxResults),
          matchCount: matchingRows.length,
          searchTerms,
        });
      }
    }

    // Sort by relevance (number of matching rows)
    results.sort((a, b) => b.matchCount - a.matchCount);

    return results.slice(0, maxResults);
  }

  /**
   * Extract search terms from query
   */
  private extractSearchTerms(query: string): string[] {
    // Simple term extraction - could be enhanced with NLP
    return query
      .toLowerCase()
      .split(/\s+/)
      .filter((term) => term.length > 2)
      .filter(
        (term) =>
          ![
            "the",
            "and",
            "or",
            "but",
            "in",
            "on",
            "at",
            "to",
            "for",
            "of",
            "with",
            "by",
          ].includes(term),
      );
  }

  /**
   * Find rows that match search terms
   */
  private findMatchingRows(
    csvData: CSVData,
    searchTerms: string[],
  ): Record<string, string>[] {
    return csvData.rows.filter((row) => {
      const rowText = Object.values(row).join(" ").toLowerCase();
      return searchTerms.some((term) => rowText.includes(term));
    });
  }

  /**
   * Get all CSV source IDs
   */
  getAllSources(): string[] {
    return Array.from(this.csvSources.keys());
  }

  /**
   * Get data for a specific source
   */
  getSourceData(sourceId: string): CSVData | undefined {
    return this.csvSources.get(sourceId);
  }

  /**
   * Query a specific CSV source
   */
  async querySource(
    sourceId: string,
    query: string,
  ): Promise<CSVQueryResult | null> {
    const csvData = this.csvSources.get(sourceId);
    if (!csvData) {
      return null;
    }

    const searchTerms = this.extractSearchTerms(query);
    const matchingRows = this.findMatchingRows(csvData, searchTerms);

    return {
      sourceId,
      title: csvData.title,
      matchingRows,
      matchCount: matchingRows.length,
      searchTerms,
    };
  }
}

export const csvQueryService = new CSVQueryService();

